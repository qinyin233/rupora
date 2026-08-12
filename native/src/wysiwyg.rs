use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag, TagEnd};

use crate::markdown::parser_options;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisualStyle {
    pub heading: u8,
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: bool,
    pub marker: bool,
    pub quote: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualRun {
    pub range: Range<usize>,
    pub style: VisualStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualProjection {
    text: String,
    source_boundaries: Vec<usize>,
    runs: Vec<VisualRun>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualSourceEdit {
    pub source: String,
    pub selection: Range<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
struct FormatState {
    heading: u8,
    strong: usize,
    emphasis: usize,
    strikethrough: usize,
    code: usize,
    link: usize,
    quote: usize,
}

impl FormatState {
    fn visual(self) -> VisualStyle {
        VisualStyle {
            heading: self.heading,
            strong: self.strong > 0,
            emphasis: self.emphasis > 0,
            strikethrough: self.strikethrough > 0,
            code: self.code > 0,
            link: self.link > 0,
            marker: false,
            quote: self.quote > 0,
        }
    }
}

impl VisualProjection {
    pub fn from_markdown(source: &str) -> Self {
        let mut builder = ProjectionBuilder::new();
        let mut format = FormatState::default();
        let mut table_cells = 0usize;
        let mut block_depth = 0usize;
        let mut trailing_container_block = false;
        let mut trailing_fenced_code_block = false;

        for (event, range) in Parser::new_ext(source, parser_options()).into_offset_iter() {
            match event {
                Event::Start(tag) => {
                    if block_depth == 0 {
                        trailing_container_block =
                            matches!(&tag, Tag::List(_) | Tag::BlockQuote(_));
                        trailing_fenced_code_block =
                            matches!(&tag, Tag::CodeBlock(CodeBlockKind::Fenced(_)));
                    }
                    block_depth += 1;
                    match tag {
                        Tag::Heading { level, .. } => format.heading = heading_level(level),
                        Tag::BlockQuote(_) => format.quote += 1,
                        Tag::CodeBlock(_) | Tag::HtmlBlock => format.code += 1,
                        Tag::Item => {
                            builder.ensure_line_break(range.start, format.visual());
                            if let Some((prefix_range, prefix)) = item_prefix(source, range.start) {
                                builder.append_marker(&prefix, prefix_range, format.visual());
                            }
                        }
                        Tag::TableRow => table_cells = 0,
                        Tag::TableCell => {
                            if table_cells > 0 {
                                builder.append_virtual("  │  ", range.start, marker_style(format));
                            }
                            table_cells += 1;
                        }
                        Tag::Emphasis => format.emphasis += 1,
                        Tag::Strong => format.strong += 1,
                        Tag::Strikethrough => format.strikethrough += 1,
                        Tag::Link { .. } => format.link += 1,
                        Tag::Image { .. } => {
                            format.link += 1;
                            let marker_end = source[range.clone()]
                                .find('[')
                                .map_or(range.start, |offset| range.start + offset + 1);
                            builder.append_marker(
                                "▧ ",
                                range.start..marker_end,
                                marker_style(format),
                            );
                        }
                        Tag::FootnoteDefinition(label) => {
                            let marker = format!("〔{label}〕 ");
                            let marker_end = source[range.clone()]
                                .find(':')
                                .map_or(range.start, |offset| range.start + offset + 1);
                            builder.append_marker(
                                &marker,
                                range.start..marker_end,
                                marker_style(format),
                            );
                        }
                        _ => {}
                    }
                }
                Event::End(tag) => {
                    match tag {
                        TagEnd::Paragraph
                        | TagEnd::Heading(_)
                        | TagEnd::CodeBlock
                        | TagEnd::Item
                        | TagEnd::TableRow
                        | TagEnd::FootnoteDefinition => {
                            builder.ensure_line_break(
                                trailing_line_break_boundary(source, &range),
                                format.visual(),
                            );
                        }
                        _ => {}
                    }
                    match tag {
                        TagEnd::Heading(_) => format.heading = 0,
                        TagEnd::BlockQuote(_) => format.quote = format.quote.saturating_sub(1),
                        TagEnd::CodeBlock | TagEnd::HtmlBlock => {
                            format.code = format.code.saturating_sub(1);
                        }
                        TagEnd::Emphasis => format.emphasis = format.emphasis.saturating_sub(1),
                        TagEnd::Strong => format.strong = format.strong.saturating_sub(1),
                        TagEnd::Strikethrough => {
                            format.strikethrough = format.strikethrough.saturating_sub(1);
                        }
                        TagEnd::Link | TagEnd::Image => {
                            format.link = format.link.saturating_sub(1);
                        }
                        _ => {}
                    }
                    block_depth = block_depth.saturating_sub(1);
                }
                Event::Text(text) => {
                    builder.append_container_prefix(source, range.start, format);
                    builder.append_mapped(source, &text, range, format.visual());
                }
                Event::Code(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                    builder.append_container_prefix(source, range.start, format);
                    let mut style = format.visual();
                    style.code = true;
                    builder.append_mapped(source, &text, range, style);
                }
                Event::Html(text) => {
                    builder.append_container_prefix(source, range.start, format);
                    let rendered = strip_html_tags(&text);
                    builder.append_transformed(&rendered, range, format.visual());
                }
                Event::InlineHtml(_) => {}
                Event::FootnoteReference(label) => {
                    let rendered = format!("〔{label}〕");
                    let mut style = format.visual();
                    style.link = true;
                    builder.append_transformed(&rendered, range, style);
                }
                Event::SoftBreak | Event::HardBreak => {
                    let next_line_start = range.end;
                    builder.append_transformed("\n", range, format.visual());
                    builder.append_container_prefix_at_line_start(source, next_line_start, format);
                }
                Event::Rule => {
                    if block_depth == 0 {
                        trailing_container_block = false;
                        trailing_fenced_code_block = false;
                    }
                    builder.append_transformed(
                        "────────────────",
                        range.clone(),
                        marker_style(format),
                    );
                    builder.ensure_line_break(range.end, format.visual());
                }
                Event::TaskListMarker(_) => {}
            }
        }

        builder.append_trailing_container_line(source);
        builder.finish(source, trailing_container_block, trailing_fenced_code_block)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn visual_char_for_source_byte(&self, source_byte: usize) -> usize {
        self.source_boundaries
            .partition_point(|boundary| *boundary < source_byte)
            .min(self.char_count())
    }

    pub fn source_char_range(&self, source: &str, visual_range: Range<usize>) -> Range<usize> {
        let range = clamp_range(visual_range, self.char_count());
        let start = self.source_boundaries[range.start];
        let end = self.source_boundaries[range.end];
        source[..start].chars().count()..source[..end].chars().count()
    }

    pub fn visual_char_range(&self, source: &str, source_range: Range<usize>) -> Range<usize> {
        let source_length = source.chars().count();
        let range = clamp_range(source_range, source_length);
        self.visual_char_for_source_byte(char_to_byte(source, range.start))
            ..self.visual_char_for_source_byte(char_to_byte(source, range.end))
    }

    pub fn apply_edit(
        &self,
        source: &str,
        edited: &str,
        visual_selection: Range<usize>,
    ) -> Option<VisualSourceEdit> {
        let change = text_change(&self.text, edited)?;
        let source_start = self.source_boundaries[change.old.start];
        let source_end = self.source_boundaries[change.old.end];
        let replacement_start = char_to_byte(edited, change.new.start);
        let replacement_end = char_to_byte(edited, change.new.end);
        let replacement = &edited[replacement_start..replacement_end];

        let mut output = source.to_owned();
        output.replace_range(source_start..source_end, replacement);
        let delta = replacement.len() as isize - (source_end - source_start) as isize;
        let selection = clamp_range(visual_selection, edited.chars().count());
        let start_byte = self.edited_source_byte(
            edited,
            selection.start,
            &change,
            source_start,
            source_end,
            delta,
        );
        let end_byte = self.edited_source_byte(
            edited,
            selection.end,
            &change,
            source_start,
            source_end,
            delta,
        );

        Some(VisualSourceEdit {
            selection: output[..start_byte].chars().count()..output[..end_byte].chars().count(),
            source: output,
        })
    }

    pub fn runs_for(&self, edited: &str) -> Vec<VisualRun> {
        let Some(change) = text_change(&self.text, edited) else {
            return self.runs.clone();
        };
        let delta = change.new.len() as isize - change.old.len() as isize;
        let inserted_style = self.style_at(change.old.start);
        let mut runs = Vec::with_capacity(self.runs.len() + 1);

        for run in &self.runs {
            if run.range.start < change.old.start {
                push_run(
                    &mut runs,
                    run.range.start..run.range.end.min(change.old.start),
                    run.style,
                );
            }
        }
        push_run(&mut runs, change.new.clone(), inserted_style);
        for run in &self.runs {
            if run.range.end > change.old.end {
                let start = run.range.start.max(change.old.end);
                push_run(
                    &mut runs,
                    shift_index(start, delta)..shift_index(run.range.end, delta),
                    run.style,
                );
            }
        }

        runs
    }

    fn char_count(&self) -> usize {
        self.source_boundaries.len().saturating_sub(1)
    }

    fn style_at(&self, index: usize) -> VisualStyle {
        self.runs
            .iter()
            .find(|run| run.range.contains(&index))
            .or_else(|| self.runs.iter().rev().find(|run| run.range.end <= index))
            .or_else(|| self.runs.first())
            .map_or_else(VisualStyle::default, |run| run.style)
    }

    #[allow(clippy::too_many_arguments)]
    fn edited_source_byte(
        &self,
        edited: &str,
        visual_index: usize,
        change: &TextChange,
        source_start: usize,
        source_end: usize,
        delta: isize,
    ) -> usize {
        if visual_index <= change.new.start {
            return self.source_boundaries[visual_index.min(change.old.start)];
        }
        if visual_index >= change.new.end {
            let old_index = change.old.end + visual_index - change.new.end;
            let old_byte = self.source_boundaries[old_index.min(self.char_count())];
            return if old_byte >= source_end {
                shift_index(old_byte, delta)
            } else {
                old_byte
            };
        }
        let replacement_start = char_to_byte(edited, change.new.start);
        let relative = char_to_byte(
            &edited[replacement_start..char_to_byte(edited, change.new.end)],
            visual_index - change.new.start,
        );
        source_start + relative
    }
}

pub fn complete_visual_enter(
    source: &mut String,
    selection: Range<usize>,
    shift: bool,
) -> Range<usize> {
    let mut selection = selection;
    if shift {
        let inserted = ensure_hard_break_before_cursor(source, selection.end);
        selection =
            selection.start.saturating_add(inserted)..selection.end.saturating_add(inserted);
    }
    if shift || has_hard_break_before_cursor(source, selection.end) {
        selection
    } else {
        crate::editing::continue_markdown_line(source, selection.end).unwrap_or(selection)
    }
}

fn ensure_hard_break_before_cursor(source: &mut String, cursor: usize) -> usize {
    let cursor_byte = char_to_byte(source, cursor);
    let Some(newline_start) = newline_start_before_cursor(source, cursor_byte) else {
        return 0;
    };
    if source[..newline_start].ends_with('\\') {
        return 0;
    }
    let spaces = source[..newline_start]
        .bytes()
        .rev()
        .take_while(|byte| *byte == b' ')
        .take(2)
        .count();
    let inserted = 2usize.saturating_sub(spaces);
    if inserted > 0 {
        source.insert_str(newline_start, &" ".repeat(inserted));
    }
    inserted
}

fn has_hard_break_before_cursor(source: &str, cursor: usize) -> bool {
    let cursor_byte = char_to_byte(source, cursor);
    let Some(newline_start) = newline_start_before_cursor(source, cursor_byte) else {
        return false;
    };
    source[..newline_start].ends_with("  ") || source[..newline_start].ends_with('\\')
}

fn newline_start_before_cursor(source: &str, cursor_byte: usize) -> Option<usize> {
    let before = source.get(..cursor_byte)?;
    if before.ends_with("\r\n") {
        Some(cursor_byte - 2)
    } else if before.ends_with(['\n', '\r']) {
        Some(cursor_byte - 1)
    } else {
        None
    }
}

struct ProjectionBuilder {
    text: String,
    source_boundaries: Vec<usize>,
    runs: Vec<VisualRun>,
}

impl ProjectionBuilder {
    fn new() -> Self {
        Self {
            text: String::new(),
            source_boundaries: vec![0],
            runs: Vec::new(),
        }
    }

    fn finish(
        mut self,
        source: &str,
        trailing_container_block: bool,
        trailing_fenced_code_block: bool,
    ) -> VisualProjection {
        while self.text.ends_with('\n') {
            let newline = self.text.chars().count() - 1;
            let start = self.source_boundaries[newline];
            let end = self.source_boundaries[newline + 1];
            let maps_source_line_break = source
                .get(start..end)
                .is_some_and(|mapped| mapped.bytes().any(|byte| matches!(byte, b'\n' | b'\r')));
            if maps_source_line_break {
                break;
            }
            self.text.pop();
            self.source_boundaries.pop();
        }

        if trailing_fenced_code_block && self.text.ends_with('\n') {
            // pulldown-cmark includes the mandatory line terminator before a
            // closing fence in the code text. It terminates the final code
            // line, but is not itself an editable blank line.
            self.text.pop();
            self.source_boundaries.pop();
        }

        let trailing_breaks = trailing_line_break_ranges(source);
        let collapsed_breaks = usize::from(trailing_container_block && trailing_breaks.len() >= 2);
        let mut represented_breaks = 0usize;
        let trailing_visual_start = self.text.chars().count().saturating_sub(
            self.text
                .chars()
                .rev()
                .take_while(|character| *character == '\n')
                .count(),
        );
        for visual_index in trailing_visual_start..self.text.chars().count() {
            let boundary = self.source_boundaries[visual_index + 1];
            if let Some(relative) = trailing_breaks[represented_breaks..]
                .iter()
                .position(|range| range.end == boundary)
            {
                represented_breaks += relative + 1;
            }
        }

        if collapsed_breaks == 1 && represented_breaks >= 1 {
            let collapsed_boundary = trailing_breaks[1].end;
            self.set_current_boundary(collapsed_boundary);
            represented_breaks = represented_breaks.max(2);
        }

        let trimmed_length = self.text.chars().count();
        self.runs.retain_mut(|run| {
            run.range.end = run.range.end.min(trimmed_length);
            run.range.start < run.range.end
        });
        let first_unrepresented = represented_breaks.max(collapsed_breaks);
        for line_break in trailing_breaks.iter().skip(first_unrepresented) {
            let visual_start = self.text.chars().count();
            self.text.push('\n');
            let previous_boundary = self.source_boundaries.last().copied().unwrap_or_default();
            self.source_boundaries
                .push(line_break.end.max(previous_boundary).min(source.len()));
            push_run(
                &mut self.runs,
                visual_start..visual_start + 1,
                VisualStyle::default(),
            );
        }
        self.append_trailing_editable_whitespace(source);
        let length = self.text.chars().count();
        self.runs.retain_mut(|run| {
            run.range.end = run.range.end.min(length);
            run.range.start < run.range.end
        });
        VisualProjection {
            text: self.text,
            source_boundaries: self.source_boundaries,
            runs: self.runs,
        }
    }

    fn append_container_prefix(&mut self, source: &str, source_start: usize, format: FormatState) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            return;
        }
        let line_start = source[..source_start]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let raw = &source[line_start..source_start];
        let (consumed, visual) = container_prefix(raw);
        if !visual.is_empty() {
            self.append_marker(
                &visual,
                line_start..line_start + consumed,
                marker_style(format),
            );
        }
    }

    fn append_container_prefix_at_line_start(
        &mut self,
        source: &str,
        line_start: usize,
        format: FormatState,
    ) {
        if line_start >= source.len() || !self.text.ends_with('\n') {
            return;
        }
        let line_end = source[line_start..]
            .find('\n')
            .map_or(source.len(), |offset| line_start + offset);
        let (consumed, visual) = container_prefix(&source[line_start..line_end]);
        if !visual.is_empty() {
            self.append_marker(
                &visual,
                line_start..line_start + consumed,
                marker_style(format),
            );
        }
    }

    fn append_trailing_container_line(&mut self, source: &str) {
        let mut line_start = 0usize;
        let mut candidate = None;
        for line in source.split_inclusive('\n') {
            let content = line.trim_end_matches(['\r', '\n']);
            if !content.is_empty() {
                candidate = Some((line_start, content));
            }
            line_start += line.len();
        }
        if line_start < source.len() {
            let content = &source[line_start..];
            if !content.is_empty() {
                candidate = Some((line_start, content));
            }
        }
        let Some((line_start, content)) = candidate else {
            return;
        };
        let (consumed, visual) = container_prefix(content);
        let already_mapped = self
            .source_boundaries
            .last()
            .is_some_and(|boundary| *boundary >= line_start + consumed);
        if consumed == content.len() && visual.contains('│') && !already_mapped {
            let mut style = VisualStyle {
                quote: true,
                ..VisualStyle::default()
            };
            style.marker = true;
            self.ensure_line_break(line_start, style);
            self.append_marker(&visual, line_start..line_start + consumed, style);
        }
    }

    fn append_trailing_editable_whitespace(&mut self, source: &str) {
        let trailing_start = source.trim_end_matches([' ', '\t']).len();
        if trailing_start == source.len() {
            return;
        }
        if !source[..trailing_start].is_empty() && self.text.is_empty() {
            return;
        }
        let source_whitespace = &source[trailing_start..];
        let visible_whitespace = self
            .text
            .bytes()
            .rev()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count()
            .min(source_whitespace.len());
        if visible_whitespace == source_whitespace.len() {
            return;
        }
        let unmapped_start = trailing_start + visible_whitespace;
        self.set_current_boundary(unmapped_start);

        let style = self
            .runs
            .iter()
            .rev()
            .find(|run| !run.style.marker)
            .map_or_else(VisualStyle::default, |run| run.style);
        self.append_mapped(
            source,
            &source[unmapped_start..],
            unmapped_start..source.len(),
            style,
        );
    }

    fn append_mapped(
        &mut self,
        source: &str,
        rendered: &str,
        range: Range<usize>,
        style: VisualStyle,
    ) {
        if rendered.is_empty() {
            return;
        }
        let fragment = &source[range.clone()];
        if let Some(relative_start) = fragment.find(rendered) {
            let source_start = range.start + relative_start;
            self.set_current_boundary(source_start);
            let visual_start = self.text.chars().count();
            self.text.push_str(rendered);
            let mut consumed = 0usize;
            for character in rendered.chars() {
                consumed += character.len_utf8();
                self.source_boundaries.push(source_start + consumed);
            }
            push_run(
                &mut self.runs,
                visual_start..self.text.chars().count(),
                style,
            );
            return;
        }
        self.append_transformed(rendered, range, style);
    }

    fn append_transformed(
        &mut self,
        rendered: &str,
        source_range: Range<usize>,
        style: VisualStyle,
    ) {
        if rendered.is_empty() {
            self.set_current_boundary(source_range.end);
            return;
        }
        self.set_current_boundary(source_range.start);
        let rendered_chars = rendered.chars().count();
        let visual_start = self.text.chars().count();
        self.text.push_str(rendered);
        for index in 1..=rendered_chars {
            self.source_boundaries.push(if index == rendered_chars {
                source_range.end
            } else {
                source_range.start
            });
        }
        push_run(
            &mut self.runs,
            visual_start..self.text.chars().count(),
            style,
        );
    }

    fn append_marker(
        &mut self,
        rendered: &str,
        source_range: Range<usize>,
        mut style: VisualStyle,
    ) {
        style.marker = true;
        self.append_transformed(rendered, source_range, style);
    }

    fn append_virtual(&mut self, rendered: &str, source_byte: usize, style: VisualStyle) {
        self.append_transformed(rendered, source_byte..source_byte, style);
    }

    fn ensure_line_break(&mut self, source_byte: usize, style: VisualStyle) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            let start = self.text.chars().count();
            self.text.push('\n');
            self.source_boundaries.push(source_byte);
            push_run(&mut self.runs, start..start + 1, style);
        }
    }

    fn set_current_boundary(&mut self, source_byte: usize) {
        if let Some(boundary) = self.source_boundaries.last_mut() {
            *boundary = source_byte;
        }
    }
}

#[derive(Clone, Debug)]
struct TextChange {
    old: Range<usize>,
    new: Range<usize>,
}

fn text_change(before: &str, after: &str) -> Option<TextChange> {
    if before == after {
        return None;
    }
    let before_length = before.chars().count();
    let after_length = after.chars().count();
    let prefix = before
        .chars()
        .zip(after.chars())
        .take_while(|(left, right)| left == right)
        .count();
    let maximum_suffix = before_length.min(after_length).saturating_sub(prefix);
    let suffix = before
        .chars()
        .rev()
        .zip(after.chars().rev())
        .take(maximum_suffix)
        .take_while(|(left, right)| left == right)
        .count();
    Some(TextChange {
        old: prefix..before_length - suffix,
        new: prefix..after_length - suffix,
    })
}

fn item_prefix(source: &str, item_start: usize) -> Option<(Range<usize>, String)> {
    let line_start = source[..item_start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = source[item_start..]
        .find('\n')
        .map_or(source.len(), |offset| item_start + offset);
    let line = &source[line_start..line_end];
    let marker_offset = item_start - line_start;
    let mut visual = container_prefix(&line[..marker_offset]).1;
    let marker = &line[marker_offset..];
    let mut cursor;

    if marker.starts_with(['-', '+', '*']) {
        cursor = 1;
        cursor += marker[cursor..]
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        if cursor == 1 {
            return None;
        }
        if marker[cursor..].starts_with("[ ]") || marker[cursor..].starts_with("[x]") {
            let checked = marker[cursor..].starts_with("[x]");
            cursor += 3;
            cursor += marker[cursor..]
                .bytes()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count();
            visual.push_str(if checked { "☑ " } else { "☐ " });
        } else {
            visual.push_str("• ");
        }
    } else {
        let digits = marker.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || !matches!(marker.as_bytes().get(digits), Some(b'.' | b')')) {
            return None;
        }
        cursor = digits + 1;
        let spaces = marker[cursor..]
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        if spaces == 0 {
            return None;
        }
        cursor += spaces;
        visual.push_str(&marker[..digits]);
        visual.push_str(". ");
    }

    Some((line_start..item_start + cursor, visual))
}

fn container_prefix(raw: &str) -> (usize, String) {
    let mut cursor = 0usize;
    let mut visual = String::new();
    loop {
        let whitespace = raw[cursor..]
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        visual.push_str(&raw[cursor..cursor + whitespace]);
        cursor += whitespace;
        if raw[cursor..].starts_with("> ") {
            visual.push_str("│ ");
            cursor += 2;
        } else {
            break;
        }
    }
    (cursor, visual)
}

fn strip_html_tags(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut inside_tag = false;
    for character in source.chars() {
        match character {
            '<' => inside_tag = true,
            '>' if inside_tag => inside_tag = false,
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }
    output
}

fn trailing_line_break_boundary(source: &str, range: &Range<usize>) -> usize {
    let mut content_end = range.end;
    while content_end > range.start && matches!(source.as_bytes()[content_end - 1], b'\n' | b'\r') {
        content_end -= 1;
    }
    if source.as_bytes().get(content_end) == Some(&b'\r')
        && source.as_bytes().get(content_end + 1) == Some(&b'\n')
    {
        (content_end + 2).min(range.end)
    } else if matches!(source.as_bytes().get(content_end), Some(b'\n' | b'\r')) {
        (content_end + 1).min(range.end)
    } else {
        range.end
    }
}

fn trailing_line_break_ranges(source: &str) -> Vec<Range<usize>> {
    let mut cursor = source.trim_end_matches([' ', '\t']).len();
    let bytes = source.as_bytes();
    let mut ranges = Vec::new();
    while cursor > 0 {
        let end = cursor;
        let mut start = cursor - 1;
        match bytes[start] {
            b'\n' => {
                if start > 0 && bytes[start - 1] == b'\r' {
                    start -= 1;
                }
            }
            b'\r' => {}
            _ => break,
        }
        ranges.push(start..end);
        cursor = start;
    }
    ranges.reverse();
    ranges
}

fn marker_style(format: FormatState) -> VisualStyle {
    let mut style = format.visual();
    style.marker = true;
    style
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn push_run(runs: &mut Vec<VisualRun>, range: Range<usize>, style: VisualStyle) {
    if range.is_empty() {
        return;
    }
    if let Some(previous) = runs.last_mut()
        && previous.range.end == range.start
        && previous.style == style
    {
        previous.range.end = range.end;
    } else {
        runs.push(VisualRun { range, style });
    }
}

fn clamp_range(range: Range<usize>, length: usize) -> Range<usize> {
    range.start.min(length).min(range.end)..range.start.max(range.end).min(length)
}

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

fn shift_index(index: usize, delta: isize) -> usize {
    if delta >= 0 {
        index.saturating_add(delta as usize)
    } else {
        index.saturating_sub(delta.unsigned_abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn projects_common_markdown_without_exposing_source_markers() {
        let source = "# 标题 **粗体**\n\n- 第一项\n- [x] 第二项 *强调*\n  - 子项\n\n> 引用 `代码`";
        let projection = VisualProjection::from_markdown(source);

        assert_eq!(
            projection.text(),
            "标题 粗体\n• 第一项\n☑ 第二项 强调\n  • 子项\n│ 引用 代码"
        );
        assert!(!projection.text().contains("**"));
        assert!(projection.runs.iter().any(|run| run.style.heading == 1));
        assert!(projection.runs.iter().any(|run| run.style.strong));
        assert!(projection.runs.iter().any(|run| run.style.emphasis));
        assert!(projection.runs.iter().any(|run| run.style.code));
        assert!(projection.runs.iter().any(|run| run.style.marker));
    }

    #[test]
    fn edits_formatted_text_without_destroying_its_markdown() {
        let source = "这里有 **粗体** 文本";
        let projection = VisualProjection::from_markdown(source);
        let edited = projection.text().replace("粗体", "醒目的粗体");
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();

        assert_eq!(update.source, "这里有 **醒目的粗体** 文本");
        assert_eq!(update.selection, 16..16);
    }

    #[test]
    fn maps_visual_list_content_back_to_source_characters() {
        let source = "- 第一项\n- [ ] 第二项";
        let projection = VisualProjection::from_markdown(source);
        let second = projection.text().find("第二项").unwrap();
        let visual_start = projection.text()[..second].chars().count();
        let source_range = projection.source_char_range(source, visual_start..visual_start + 3);

        assert_eq!(
            &source[char_to_byte(source, source_range.start)..],
            "第二项"
        );
    }

    #[test]
    fn inserted_text_inherits_the_surrounding_visual_style() {
        let projection = VisualProjection::from_markdown("**粗体**");
        let runs = projection.runs_for("粗体字");

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].range, 0..3);
        assert!(runs[0].style.strong);
    }

    #[test]
    fn enter_continues_visual_lists_tasks_and_quotes_without_showing_markdown() {
        for (source, expected_source, expected_visual) in [
            ("- 项目", "- 项目\n- ", "• 项目\n• "),
            ("3. 项目", "3. 项目\n4. ", "3. 项目\n4. "),
            ("- [x] 完成", "- [x] 完成\n- [ ] ", "☑ 完成\n☐ "),
            ("> 引用", "> 引用\n> ", "│ 引用\n│ "),
        ] {
            let projection = VisualProjection::from_markdown(source);
            let edited = format!("{}\n", projection.text());
            let cursor = edited.chars().count();
            let mut update = projection
                .apply_edit(source, &edited, cursor..cursor)
                .unwrap();
            update.selection = complete_visual_enter(&mut update.source, update.selection, false);

            assert_eq!(update.source, expected_source);
            let continued = VisualProjection::from_markdown(&update.source);
            assert_eq!(continued.text(), expected_visual);
            let source_cursor = char_to_byte(&update.source, update.selection.end);
            assert_eq!(
                continued.visual_char_for_source_byte(source_cursor),
                expected_visual.chars().count()
            );
        }
    }

    #[test]
    fn preserves_trailing_spaces_until_the_user_completes_a_hard_break() {
        for source in ["普通段落  ", "- 列表正文  ", "**粗体**  "] {
            let projection = VisualProjection::from_markdown(source);
            assert!(projection.text().ends_with("  "), "source: {source:?}");
            assert_eq!(
                projection.source_boundaries.last().copied(),
                Some(source.len()),
                "source: {source:?}"
            );
        }

        let source = "- 列表正文";
        let projection = VisualProjection::from_markdown(source);
        let edited = format!("{}  ", projection.text());
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, "- 列表正文  ");
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited
        );
    }

    #[test]
    fn two_spaces_then_enter_stays_inside_the_current_list_item() {
        let source = "- 列表正文  ";
        let projection = VisualProjection::from_markdown(source);
        let edited = format!("{}\n", projection.text());
        let cursor = edited.chars().count();
        let mut update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        update.selection = complete_visual_enter(&mut update.source, update.selection, false);

        assert_eq!(update.source, "- 列表正文  \n");
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            "• 列表正文\n"
        );
    }

    #[test]
    fn shift_enter_inserts_a_hard_break_without_starting_a_new_list_item() {
        let source = "- 列表正文";
        let projection = VisualProjection::from_markdown(source);
        let edited = format!("{}\n", projection.text());
        let cursor = edited.chars().count();
        let mut update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        update.selection = complete_visual_enter(&mut update.source, update.selection, true);

        assert_eq!(update.source, "- 列表正文  \n");
        assert_eq!(
            update.selection,
            update.source.chars().count()..update.source.chars().count()
        );
    }

    #[test]
    fn repeated_space_frames_survive_until_enter_and_accept_continuation_text() {
        let mut source = "- 一行文字".to_owned();
        for spaces in 1..=3 {
            let projection = VisualProjection::from_markdown(&source);
            let mut edited = projection.text().to_owned();
            edited.push(' ');
            let cursor = edited.chars().count();
            let update = projection
                .apply_edit(&source, &edited, cursor..cursor)
                .unwrap();
            source = update.source;
            assert_eq!(source, format!("- 一行文字{}", " ".repeat(spaces)));
            assert_eq!(VisualProjection::from_markdown(&source).text(), edited);
        }

        let projection = VisualProjection::from_markdown(&source);
        let mut edited = projection.text().to_owned();
        edited.push('\n');
        let cursor = edited.chars().count();
        let mut update = projection
            .apply_edit(&source, &edited, cursor..cursor)
            .unwrap();
        update.selection = complete_visual_enter(&mut update.source, update.selection, false);
        assert_eq!(update.source, "- 一行文字   \n");
        assert!(!update.source.contains("\n- "));

        source = update.source;
        let projection = VisualProjection::from_markdown(&source);
        let mut edited = projection.text().to_owned();
        edited.push_str("继续输入");
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(&source, &edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, "- 一行文字   \n继续输入");
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited
        );
    }

    #[test]
    fn repeated_enter_frames_preserve_every_plain_blank_line() {
        let mut source = "第一段".to_owned();
        for line_breaks in 1..=4 {
            let projection = VisualProjection::from_markdown(&source);
            let mut edited = projection.text().to_owned();
            edited.push('\n');
            let cursor = edited.chars().count();
            let mut update = projection
                .apply_edit(&source, &edited, cursor..cursor)
                .unwrap();
            update.selection = complete_visual_enter(&mut update.source, update.selection, false);
            source = update.source;

            let expected = format!("第一段{}", "\n".repeat(line_breaks));
            assert_eq!(source, expected);
            assert_eq!(VisualProjection::from_markdown(&source).text(), expected);
        }

        let projection = VisualProjection::from_markdown(&source);
        let mut edited = projection.text().to_owned();
        edited.push_str("第二段");
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(&source, &edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, "第一段\n\n\n\n第二段");
    }

    #[test]
    fn empty_formatted_and_crlf_documents_keep_trailing_blank_lines() {
        for (source, expected) in [
            ("\n\n\n", "\n\n\n"),
            ("**粗体**\n\n\n", "粗体\n\n\n"),
            ("段落\r\n\r\n\r\n", "段落\n\n\n"),
        ] {
            let projection = VisualProjection::from_markdown(source);
            assert_eq!(projection.text(), expected, "source: {source:?}");
            assert_eq!(
                projection.source_boundaries.last().copied(),
                Some(source.len()),
                "source: {source:?}"
            );
        }
    }

    #[test]
    fn fenced_code_keeps_blank_content_without_exposing_the_closing_terminator() {
        for (source, expected) in [
            ("```text\n代码\n```", "代码"),
            ("```text\n代码\n\n```", "代码\n"),
            ("```text\n代码\n```\n\n", "代码\n\n"),
        ] {
            assert_eq!(
                VisualProjection::from_markdown(source).text(),
                expected,
                "source: {source:?}"
            );
        }
    }

    #[test]
    fn list_exit_collapses_only_its_structural_separator() {
        let source = "- 项目\n\n";
        let projection = VisualProjection::from_markdown(source);
        assert_eq!(projection.text(), "• 项目\n");

        let edited = format!("{}\n", projection.text());
        let cursor = edited.chars().count();
        let mut update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        update.selection = complete_visual_enter(&mut update.source, update.selection, false);
        assert_eq!(update.source, "- 项目\n\n\n");
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            "• 项目\n\n"
        );
    }

    #[test]
    fn backspace_removes_exactly_one_of_several_blank_lines() {
        let source = "段落\n\n\n";
        let projection = VisualProjection::from_markdown(source);
        let mut edited = projection.text().to_owned();
        edited.pop();
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();

        assert_eq!(update.source, "段落\n\n");
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited
        );
        assert_eq!(update.selection, cursor..cursor);
    }

    #[test]
    fn backspace_removes_one_trailing_space_without_collapsing_the_cursor() {
        let source = "- 文本  ";
        let projection = VisualProjection::from_markdown(source);
        let mut edited = projection.text().to_owned();
        edited.pop();
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();

        assert_eq!(update.source, "- 文本 ");
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited
        );
        assert_eq!(update.selection, cursor..cursor);
    }

    #[test]
    fn heading_enter_creates_a_plain_visual_paragraph() {
        let source = "# 标题";
        let projection = VisualProjection::from_markdown(source);
        let edited = "标题\n正文";
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();

        assert_eq!(update.source, "# 标题\n正文");
        let updated = VisualProjection::from_markdown(&update.source);
        assert_eq!(updated.text(), edited);
        let runs = updated.runs_for(updated.text());
        assert!(runs.iter().any(|run| run.style.heading == 1));
        assert!(runs.iter().any(|run| run.style.heading == 0));
    }

    #[test]
    fn keeps_one_editable_blank_line_after_exiting_a_list() {
        let source = "- 第一项\n\n";
        let projection = VisualProjection::from_markdown(source);

        assert_eq!(projection.text(), "• 第一项\n");
        let source_cursor = source[..source.len() - 1].chars().count();
        assert_eq!(
            projection.visual_char_for_source_byte(char_to_byte(source, source_cursor)),
            projection.text().chars().count()
        );

        let edited = format!("{}列表后的普通段落", projection.text());
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, "- 第一项\n\n列表后的普通段落");
    }

    #[test]
    fn keeps_the_visual_cursor_after_a_single_heading_newline() {
        let source = "# 标题\n";
        let projection = VisualProjection::from_markdown(source);

        assert_eq!(projection.text(), "标题\n");
        assert_eq!(
            projection.visual_char_for_source_byte(source.len()),
            projection.text().chars().count()
        );

        let mut edited = projection.text().to_owned();
        edited.push_str("正文");
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, "# 标题\n正文");
    }

    proptest! {
        #[test]
        fn visual_boundaries_and_unicode_edits_stay_utf8_safe(source in any::<String>()) {
            let projection = VisualProjection::from_markdown(&source);
            prop_assert_eq!(
                projection.source_boundaries.len(),
                projection.text().chars().count() + 1
            );
            prop_assert!(
                projection
                    .source_boundaries
                    .windows(2)
                    .all(|pair| pair[0] <= pair[1])
            );
            let boundaries_are_valid = projection.source_boundaries.iter().all(|boundary| {
                *boundary <= source.len() && source.is_char_boundary(*boundary)
            });
            prop_assert!(boundaries_are_valid);

            let insertion = projection.text().chars().count() / 2;
            let mut edited = projection.text().to_owned();
            edited.insert(char_to_byte(&edited, insertion), '🙂');
            let selection = insertion + 1..insertion + 1;
            if let Some(update) = projection.apply_edit(&source, &edited, selection) {
                prop_assert!(update.selection.end <= update.source.chars().count());
                prop_assert!(update.selection.start <= update.selection.end);
            }
        }
    }
}
