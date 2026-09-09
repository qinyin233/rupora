use crate::editing::char_to_byte;

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
    pub footnote: bool,
    pub table: bool,
    pub table_header: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualRun {
    pub range: Range<usize>,
    pub style: VisualStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualProjection {
    text: String,
    source_left_boundaries: Vec<usize>,
    source_boundaries: Vec<usize>,
    runs: Vec<VisualRun>,
    atomic_ranges: Vec<AtomicVisualRange>,
    inline_wrappers: Vec<InlineWrapper>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AtomicVisualRange {
    visual: Range<usize>,
    source: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InlineWrapper {
    visual: Range<usize>,
    source: Range<usize>,
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
    table: usize,
    table_header: usize,
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
            footnote: false,
            table: self.table > 0,
            table_header: self.table_header > 0,
        }
    }
}

impl VisualProjection {
    pub fn from_markdown(source: &str) -> Self {
        Self::from_markdown_with_selection(source, None)
    }

    pub fn from_markdown_with_selection(
        source: &str,
        source_selection: Option<Range<usize>>,
    ) -> Self {
        let mut builder = ProjectionBuilder::new();
        let mut format = FormatState::default();
        let mut table_cells = 0usize;
        let mut block_depth = 0usize;
        let mut trailing_container_block = false;
        let mut trailing_fenced_code_block = false;
        let mut revealed_inline_depth = None::<usize>;
        let source_selection = source_selection.map(|selection| {
            let selection = clamp_range(selection, source.chars().count());
            char_to_byte(source, selection.start)..char_to_byte(source, selection.end)
        });
        let collapsed_source_cursor = source_selection
            .as_ref()
            .filter(|selection| selection.is_empty())
            .map(|selection| selection.start);
        let standalone_footnotes = standalone_footnote_references(source);

        for (event, range) in Parser::new_ext(source, parser_options()).into_offset_iter() {
            if let Some(depth) = revealed_inline_depth.as_mut() {
                match &event {
                    Event::Start(_) => *depth += 1,
                    Event::End(_) if *depth == 1 => revealed_inline_depth = None,
                    Event::End(_) => *depth -= 1,
                    _ => {}
                }
                continue;
            }
            match event {
                Event::Start(tag) => {
                    if block_depth == 0 && matches!(&tag, Tag::List(_) | Tag::BlockQuote(_)) {
                        // A loose block separator is represented by the surrounding block
                        // spacing. Do not let the first nested item mistake that separator for
                        // an editable blank line inside the container.
                        builder.set_current_boundary(range.start);
                    }
                    builder.append_source_line_breaks_until(source, range.start, format.visual());
                    if matches!(&tag, Tag::Link { .. } | Tag::Image { .. })
                        && source_selection.as_ref().is_some_and(|selection| {
                            selection_reveals_hidden_source(selection, &range)
                        })
                    {
                        let mut style = format.visual();
                        style.link = true;
                        style.marker = true;
                        builder.append_mapped(source, &source[range.clone()], range, style);
                        revealed_inline_depth = Some(1);
                        continue;
                    }
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
                        Tag::CodeBlock(CodeBlockKind::Fenced(_)) => {
                            format.code += 1;
                            builder.set_current_boundary(fenced_code_content_start(
                                source,
                                range.start,
                            ));
                        }
                        Tag::CodeBlock(_) | Tag::HtmlBlock => format.code += 1,
                        Tag::Item => {
                            builder.ensure_line_break(range.start, format.visual());
                            if let Some((prefix_range, prefix)) = item_prefix(source, range.start) {
                                builder.append_marker(
                                    source,
                                    &prefix,
                                    prefix_range,
                                    format.visual(),
                                );
                            }
                        }
                        Tag::Table(_) => format.table += 1,
                        Tag::TableHead => format.table_header += 1,
                        Tag::TableRow => table_cells = 0,
                        Tag::TableCell => {
                            if table_cells > 0 {
                                if let Some(separator) = table_separator_range(source, range.start)
                                {
                                    builder.append_marker(
                                        source,
                                        "  │  ",
                                        separator,
                                        marker_style(format),
                                    );
                                } else {
                                    builder.append_virtual(
                                        "  │  ",
                                        range.start,
                                        marker_style(format),
                                    );
                                }
                            }
                            table_cells += 1;
                        }
                        Tag::Emphasis => {
                            builder.begin_inline_wrapper(range.start);
                            format.emphasis += 1;
                        }
                        Tag::Strong => {
                            builder.begin_inline_wrapper(range.start);
                            format.strong += 1;
                        }
                        Tag::Strikethrough => {
                            builder.begin_inline_wrapper(range.start);
                            format.strikethrough += 1;
                        }
                        Tag::Link { .. } => {
                            builder.begin_inline_wrapper(range.start);
                            format.link += 1;
                        }
                        Tag::Image { .. } => {
                            format.link += 1;
                            let marker_end = source[range.clone()]
                                .find('[')
                                .map_or(range.start, |offset| range.start + offset + 1);
                            builder.append_marker(
                                source,
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
                                source,
                                &marker,
                                range.start..marker_end,
                                marker_style(format),
                            );
                        }
                        _ => {}
                    }
                }
                Event::End(tag) => {
                    if matches!(
                        tag,
                        TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link
                    ) {
                        builder.end_inline_wrapper(range.end);
                    }
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
                        TagEnd::Table => format.table = format.table.saturating_sub(1),
                        TagEnd::TableHead => {
                            format.table_header = format.table_header.saturating_sub(1);
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
                    if format.code == 0 {
                        builder.append_text_with_footnote_references(
                            source,
                            &text,
                            range,
                            format.visual(),
                            &standalone_footnotes,
                            source_selection.as_ref(),
                        );
                    } else {
                        builder.append_mapped(source, &text, range, format.visual());
                    }
                }
                Event::Code(text) => {
                    let syntax_end = range.end;
                    builder.begin_inline_wrapper(range.start);
                    builder.append_container_prefix(source, range.start, format);
                    let mut style = format.visual();
                    style.code = true;
                    let content_range = inline_code_content_range(source, &text, range.clone());
                    if source_selection.as_ref().is_some_and(|selection| {
                        selection_reveals_inline_code(selection, &content_range)
                    }) {
                        builder.append_revealed_inline_code(source, range, content_range, style);
                    } else {
                        builder.append_mapped(source, &text, range, style);
                    }
                    builder.end_inline_wrapper(syntax_end);
                }
                Event::InlineMath(text) | Event::DisplayMath(text) => {
                    builder.append_container_prefix(source, range.start, format);
                    let mut style = format.visual();
                    style.code = true;
                    builder.append_mapped(source, &text, range, style);
                }
                Event::Html(_) => {
                    builder.append_container_prefix(source, range.start, format);
                    builder.append_html(source, range, format.visual(), source_selection.as_ref());
                }
                Event::InlineHtml(_) => {
                    builder.append_html(source, range, format.visual(), source_selection.as_ref());
                }
                Event::FootnoteReference(label) => {
                    let mut style = format.visual();
                    style.link = true;
                    style.footnote = true;
                    builder.append_footnote_reference(
                        source,
                        &label,
                        range,
                        style,
                        source_selection.as_ref(),
                    );
                }
                Event::SoftBreak | Event::HardBreak => {
                    let next_line_start = range.end;
                    builder.append_transformed(source, "\n", range, format.visual());
                    builder.append_container_prefix_at_line_start(source, next_line_start, format);
                }
                Event::Rule => {
                    if block_depth == 0 {
                        trailing_container_block = false;
                        trailing_fenced_code_block = false;
                    }
                    builder.append_transformed(
                        source,
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
        let mut projection =
            builder.finish(source, trailing_container_block, trailing_fenced_code_block);
        if let Some(selection) = source_selection.as_ref()
            && selection.is_empty()
            && let Some((syntax, style)) = empty_inline_wrapper_at(source, selection.start)
        {
            projection.collapse_source_range(syntax, selection.start, style);
        }
        if let Some(source_byte) = collapsed_source_cursor {
            projection.anchor_source_cursor(source_byte);
        }
        projection
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
        let end = if range.is_empty() {
            start
        } else {
            self.source_left_boundaries[range.end]
        };
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
        let selection = clamp_range(visual_selection, edited.chars().count());
        let change = text_change_anchored_at_selection(&self.text, edited, &selection)?;
        let insertion = change.old.is_empty();
        let mut source_start = self.source_boundaries[change.old.start];
        let mut source_end = if insertion {
            source_start
        } else {
            self.source_left_boundaries[change.old.end]
        };
        let replacement_start = char_to_byte(edited, change.new.start);
        let replacement_end = char_to_byte(edited, change.new.end);
        let replacement = &edited[replacement_start..replacement_end];
        for atomic in self
            .atomic_ranges
            .iter()
            .filter(|atomic| visual_change_intersects(&change.old, &atomic.visual))
        {
            source_start = source_start.min(atomic.source.start);
            source_end = source_end.max(atomic.source.end);
        }
        for wrapper in self.inline_wrappers.iter().filter(|wrapper| {
            change.old.start <= wrapper.visual.start && change.old.end >= wrapper.visual.end
        }) {
            if replacement.is_empty() {
                source_start = source_start.min(wrapper.source.start);
                source_end = source_end.max(wrapper.source.end);
            }
        }
        if replacement.is_empty()
            && !change.old.is_empty()
            && self.source_left_boundaries[change.old.start] < source_start
            && self.source_boundaries[change.old.end] > source_end
        {
            source_start = self.source_left_boundaries[change.old.start];
            source_end = self.source_boundaries[change.old.end];
        }

        let mut output = source.to_owned();
        output.replace_range(source_start..source_end, replacement);
        let delta = replacement.len() as isize - (source_end - source_start) as isize;
        let mut start_byte = self.edited_source_byte(
            edited,
            selection.start,
            &change,
            source_start,
            source_end,
            delta,
        );
        let mut end_byte = self.edited_source_byte(
            edited,
            selection.end,
            &change,
            source_start,
            source_end,
            delta,
        );
        let repair_bytes = self.repair_inline_flanking_boundaries(
            source,
            edited,
            &change.old,
            source_start,
            source_end,
            delta,
            &mut output,
        );
        if !repair_bytes.is_empty() {
            const REPAIR: &str = "<!---->";
            start_byte += repair_bytes
                .iter()
                .filter(|repair| start_byte >= **repair)
                .count()
                * REPAIR.len();
            end_byte += repair_bytes
                .iter()
                .filter(|repair| end_byte >= **repair)
                .count()
                * REPAIR.len();
        }

        Some(VisualSourceEdit {
            selection: output[..start_byte].chars().count()..output[..end_byte].chars().count(),
            source: output,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn repair_inline_flanking_boundaries(
        &self,
        original_source: &str,
        edited_visual: &str,
        changed_visual: &Range<usize>,
        source_start: usize,
        source_end: usize,
        delta: isize,
        output: &mut String,
    ) -> Vec<usize> {
        const REPAIR: &str = "<!---->";
        let wrapper = self
            .inline_wrappers
            .iter()
            .filter(|wrapper| {
                wrapper.visual.start <= changed_visual.start
                    && changed_visual.end <= wrapper.visual.end
                    && source_start >= wrapper.source.start
                    && source_end <= wrapper.source.end
            })
            .filter(|wrapper| inline_flanking_wrapper(original_source, &wrapper.source))
            .min_by_key(|wrapper| wrapper.source.end - wrapper.source.start);
        let Some(wrapper) = wrapper else {
            return Vec::new();
        };
        if VisualProjection::from_markdown(output).text() == edited_visual {
            return Vec::new();
        }
        let start = wrapper.source.start;
        let end = shift_index(wrapper.source.end, delta);
        if !output.is_char_boundary(start) || !output.is_char_boundary(end) {
            return Vec::new();
        }
        for positions in [vec![end], vec![start], vec![start, end]] {
            let mut candidate = output.clone();
            for position in positions.iter().copied().rev() {
                candidate.insert_str(position, REPAIR);
            }
            if VisualProjection::from_markdown(&candidate).text() == edited_visual {
                *output = candidate;
                return positions;
            }
        }
        Vec::new()
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

    fn anchor_source_cursor(&mut self, source_byte: usize) {
        let visual_index = self.visual_char_for_source_byte(source_byte);
        if let Some(boundary) = self.source_boundaries.get_mut(visual_index) {
            *boundary = source_byte;
        }
        if let Some(boundary) = self.source_left_boundaries.get_mut(visual_index) {
            *boundary = source_byte;
        }
    }

    fn collapse_source_range(
        &mut self,
        source_range: Range<usize>,
        source_anchor: usize,
        _style: VisualStyle,
    ) {
        let visual_start = self.visual_char_for_source_byte(source_range.start);
        let visual_end = self.visual_char_for_source_byte(source_range.end);
        if visual_start >= visual_end {
            return;
        }
        let start_byte = char_to_byte(&self.text, visual_start);
        let end_byte = char_to_byte(&self.text, visual_end);
        self.text.replace_range(start_byte..end_byte, "");
        let removed = visual_end - visual_start;
        self.source_left_boundaries
            .splice(visual_start..=visual_end, [source_anchor]);
        self.source_boundaries
            .splice(visual_start..=visual_end, [source_anchor]);
        self.runs = shift_or_split_runs(&self.runs, visual_start..visual_end, removed);
        self.atomic_ranges.retain_mut(|atomic| {
            if atomic.visual.end <= visual_start {
                true
            } else if atomic.visual.start >= visual_end {
                atomic.visual.start -= removed;
                atomic.visual.end -= removed;
                true
            } else {
                false
            }
        });
        self.inline_wrappers.retain_mut(|wrapper| {
            if wrapper.visual.end <= visual_start {
                true
            } else if wrapper.visual.start >= visual_end {
                wrapper.visual.start -= removed;
                wrapper.visual.end -= removed;
                true
            } else {
                false
            }
        });
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
            let old_byte = self.source_boundaries[visual_index.min(change.old.start)];
            return if source_start < source_end && (source_start..source_end).contains(&old_byte) {
                source_start
            } else {
                old_byte
            };
        }
        if visual_index >= change.new.end {
            let old_index = change.old.end + visual_index - change.new.end;
            let old_byte = self.source_boundaries[old_index.min(self.char_count())];
            return if source_start < source_end && (source_start..=source_end).contains(&old_byte) {
                shift_index(source_end, delta)
            } else if old_byte >= source_end {
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

fn inline_flanking_wrapper(source: &str, range: &Range<usize>) -> bool {
    source.get(range.clone()).is_some_and(|fragment| {
        ["***", "___", "**", "__", "~~", "*", "_"]
            .into_iter()
            .any(|delimiter| {
                fragment.len() >= delimiter.len() * 2
                    && fragment.starts_with(delimiter)
                    && fragment.ends_with(delimiter)
            })
    })
}

fn empty_inline_wrapper_at(
    source: &str,
    source_cursor: usize,
) -> Option<(Range<usize>, VisualStyle)> {
    for (delimiter, style) in [
        (
            "**",
            VisualStyle {
                strong: true,
                ..VisualStyle::default()
            },
        ),
        (
            "~~",
            VisualStyle {
                strikethrough: true,
                ..VisualStyle::default()
            },
        ),
        (
            "*",
            VisualStyle {
                emphasis: true,
                ..VisualStyle::default()
            },
        ),
        (
            "`",
            VisualStyle {
                code: true,
                ..VisualStyle::default()
            },
        ),
    ] {
        let Some(start) = source_cursor.checked_sub(delimiter.len()) else {
            continue;
        };
        let Some(end) = source_cursor.checked_add(delimiter.len()) else {
            continue;
        };
        if source.get(start..source_cursor) == Some(delimiter)
            && source.get(source_cursor..end) == Some(delimiter)
        {
            return Some((start..end, style));
        }
    }
    None
}

fn shift_or_split_runs(
    runs: &[VisualRun],
    removed: Range<usize>,
    removed_length: usize,
) -> Vec<VisualRun> {
    let mut output = Vec::with_capacity(runs.len());
    for run in runs {
        if run.range.end <= removed.start {
            push_run(&mut output, run.range.clone(), run.style);
        } else if run.range.start >= removed.end {
            push_run(
                &mut output,
                run.range.start - removed_length..run.range.end - removed_length,
                run.style,
            );
        } else {
            if run.range.start < removed.start {
                push_run(&mut output, run.range.start..removed.start, run.style);
            }
            if run.range.end > removed.end {
                push_run(
                    &mut output,
                    removed.start..run.range.end - removed_length,
                    run.style,
                );
            }
        }
    }
    output
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

pub fn complete_fenced_code_on_enter(
    source: &mut String,
    selection: Range<usize>,
) -> Option<Range<usize>> {
    if !selection.is_empty() {
        return None;
    }
    let cursor_byte = char_to_byte(source, selection.end);
    if cursor_byte != source.len() {
        return None;
    }
    let before = source.get(..cursor_byte)?;
    let newline_start = newline_start_before_cursor(source, cursor_byte)?;
    let line_start = before[..newline_start]
        .rfind(['\n', '\r'])
        .map_or(0, |index| index + 1);
    let line = &before[line_start..newline_start];
    let trimmed = line.trim_start_matches([' ', '\t']);
    let fence_character = trimmed.chars().next()?;
    if !matches!(fence_character, '`' | '~') {
        return None;
    }
    let fence_length = trimmed
        .chars()
        .take_while(|character| *character == fence_character)
        .count();
    if fence_length < 3 {
        return None;
    }
    let indentation = &line[..line.len() - trimmed.len()];
    let closing = fence_character.to_string().repeat(fence_length);
    source.insert_str(cursor_byte, &format!("\n{indentation}{closing}"));
    Some(selection)
}

/// Turns a freshly typed bare fence into an immediately editable code block.
///
/// A visual editor cannot leave the cursor on Markdown's hidden info-string
/// line: the next text would be accepted by the source parser but remain
/// invisible. Pairing the fence as soon as its third marker is typed places
/// the cursor on the first code line instead.
pub fn complete_bare_fenced_code_after_typing(
    source: &mut String,
    selection: Range<usize>,
) -> Option<Range<usize>> {
    if !selection.is_empty() {
        return None;
    }
    // egui can report the pre-reflow cursor for the frame in which the third
    // marker turns a plain TextEdit into a fenced-code TextEdit. The source is
    // authoritative here: a bare final fence is unambiguous even if that
    // transient cursor still points one character earlier.
    let cursor_byte = source.len();
    let line_start = source[..cursor_byte]
        .rfind(['\n', '\r'])
        .map_or(0, |index| index + 1);
    if has_unclosed_fenced_code(&source[..line_start]) {
        return None;
    }
    let line = &source[line_start..cursor_byte];
    let (indentation, fence_character, fence_length, info) = parse_fence_line(line)?;
    if fence_length < 3 || !info.is_empty() {
        return None;
    }

    let closing = fence_character.to_string().repeat(fence_length);
    source.insert_str(cursor_byte, &format!("\n\n{indentation}{closing}"));
    let body_cursor = source[..cursor_byte].chars().count() + 1;
    Some(body_cursor..body_cursor)
}

/// Consumes a closing fence typed in front of the hidden auto-paired fence and
/// leaves the caret in a normal paragraph after the code block.
pub fn consume_paired_fenced_code_closer(
    source: &mut String,
    _selection: Range<usize>,
) -> Option<Range<usize>> {
    // The third marker changes the parser's block structure in the same frame,
    // so egui may still report a cursor or selection from the previous
    // two-marker line. Detect the unambiguous pair of adjacent matching
    // closers at EOF from the source instead.
    let duplicate_end = source.trim_end_matches([' ', '\t']).len();
    let duplicate_start = source[..duplicate_end]
        .rfind(['\n', '\r'])
        .map_or(0, |index| index + 1);
    let duplicate_break = line_break_before_byte(source, duplicate_start)?;
    let typed_end = duplicate_break.start;
    let typed_start = source[..typed_end]
        .rfind(['\n', '\r'])
        .map_or(0, |index| index + 1);

    let (_, typed_character, typed_length, typed_info) =
        parse_fence_line(&source[typed_start..typed_end])?;
    if !typed_info.is_empty() {
        return None;
    }
    let opening_end = source.find(['\n', '\r'])?;
    let (_, opening_character, opening_length, _) = parse_fence_line(&source[..opening_end])?;
    if typed_character != opening_character || typed_length < opening_length {
        return None;
    }

    let (_, duplicate_character, duplicate_length, duplicate_info) =
        parse_fence_line(&source[duplicate_start..duplicate_end])?;
    if duplicate_character != opening_character
        || duplicate_length < opening_length
        || !duplicate_info.is_empty()
        || !source[duplicate_end..].trim().is_empty()
    {
        return None;
    }

    source.replace_range(typed_end.., "\n\n");
    let cursor = source.chars().count();
    Some(cursor..cursor)
}

/// Makes the paragraph after a completed fenced block addressable by keyboard.
pub fn paragraph_after_fenced_code(source: &mut String) -> Option<Range<usize>> {
    let opening_end = source.find(['\n', '\r'])?;
    let (_, opening_character, opening_length, _) = parse_fence_line(&source[..opening_end])?;
    let mut line_start = skip_one_line_break(source, opening_end)?;
    while line_start <= source.len() {
        let line_end = source[line_start..]
            .find(['\n', '\r'])
            .map_or(source.len(), |offset| line_start + offset);
        if let Some((_, character, length, info)) = parse_fence_line(&source[line_start..line_end])
            && character == opening_character
            && length >= opening_length
            && info.is_empty()
            && source[line_end..].trim().is_empty()
        {
            let first_line_start = if let Some(start) = skip_one_line_break(source, line_end) {
                start
            } else {
                source.push_str("\n\n");
                let cursor = source.chars().count();
                return Some(cursor..cursor);
            };
            let paragraph_start = if let Some(start) = skip_one_line_break(source, first_line_start)
            {
                start
            } else {
                source.insert(first_line_start, '\n');
                first_line_start + 1
            };
            let cursor = source[..paragraph_start].chars().count();
            return Some(cursor..cursor);
        }
        if line_end == source.len() {
            break;
        }
        line_start = skip_one_line_break(source, line_end)?;
    }
    None
}

pub fn fenced_code_language(source: &str) -> Option<&str> {
    let opening_end = source.find(['\n', '\r']).unwrap_or(source.len());
    let (_, _, _, info) = parse_fence_line(&source[..opening_end])?;
    (!info.is_empty()).then_some(info)
}

/// Returns only the editable body of a complete fenced code block.
///
/// The opening/closing markers and the structural line break immediately
/// before the closing marker are intentionally excluded. This is the text a
/// user expects the code-block copy button to place on the clipboard.
pub fn fenced_code_content(source: &str) -> Option<&str> {
    let opening_end = source.find(['\n', '\r'])?;
    let (_, opening_character, opening_length, _) = parse_fence_line(&source[..opening_end])?;
    if opening_length < 3 {
        return None;
    }
    let body_start = skip_one_line_break(source, opening_end)?;
    let mut line_start = body_start;
    while line_start <= source.len() {
        let line_end = source[line_start..]
            .find(['\n', '\r'])
            .map_or(source.len(), |offset| line_start + offset);
        if let Some((_, character, length, info)) = parse_fence_line(&source[line_start..line_end])
            && character == opening_character
            && length >= opening_length
            && info.is_empty()
        {
            let content_end = if line_start > body_start {
                line_break_before_byte(source, line_start)
                    .filter(|line_break| line_break.start >= body_start)
                    .map_or(line_start, |line_break| line_break.start)
            } else {
                line_start
            };
            return source.get(body_start..content_end);
        }
        if line_end == source.len() {
            break;
        }
        line_start = skip_one_line_break(source, line_end)?;
    }
    None
}

pub fn move_across_hidden_inline_code_boundary(
    source: &str,
    selection: Range<usize>,
    move_left: bool,
    move_right: bool,
) -> Option<Range<usize>> {
    if !selection.is_empty() || move_left == move_right {
        return None;
    }
    let selection = clamp_range(selection, source.chars().count());
    let cursor_byte = char_to_byte(source, selection.start);

    Parser::new_ext(source, parser_options())
        .into_offset_iter()
        .find_map(|(event, syntax_range)| {
            if !matches!(event, Event::Code(_)) {
                return None;
            }
            let content_range = inline_code_delimited_content_range(source, syntax_range.clone())?;
            let target_byte = if move_left && cursor_byte == syntax_range.end {
                content_range.end
            } else if move_right && cursor_byte == syntax_range.start {
                content_range.start
            } else {
                return None;
            };
            let target = source[..target_byte].chars().count();
            Some(target..target)
        })
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

fn line_break_before_byte(source: &str, byte_index: usize) -> Option<Range<usize>> {
    let before = source.get(..byte_index)?;
    if before.ends_with("\r\n") {
        Some(byte_index - 2..byte_index)
    } else if before.ends_with(['\n', '\r']) {
        Some(byte_index - 1..byte_index)
    } else {
        None
    }
}

fn parse_fence_line(line: &str) -> Option<(&str, char, usize, &str)> {
    let indentation_length = line.bytes().take_while(|byte| *byte == b' ').count();
    if indentation_length > 3 || line.as_bytes().get(indentation_length) == Some(&b'\t') {
        return None;
    }
    let indentation = &line[..indentation_length];
    let marker_text = &line[indentation_length..];
    let fence_character = marker_text.chars().next()?;
    if !matches!(fence_character, '`' | '~') {
        return None;
    }
    let fence_length = marker_text
        .chars()
        .take_while(|character| *character == fence_character)
        .count();
    let marker_bytes = fence_character.len_utf8() * fence_length;
    let info = marker_text[marker_bytes..].trim();
    if fence_character == '`' && info.contains('`') {
        return None;
    }
    Some((indentation, fence_character, fence_length, info))
}

fn skip_one_line_break(source: &str, at: usize) -> Option<usize> {
    match source.as_bytes().get(at..) {
        Some([b'\r', b'\n', ..]) => Some(at + 2),
        Some([b'\r' | b'\n', ..]) => Some(at + 1),
        _ => None,
    }
}

fn has_unclosed_fenced_code(source: &str) -> bool {
    let mut open = None::<(char, usize)>;
    for line in source.lines() {
        let Some((_, character, length, info)) = parse_fence_line(line) else {
            continue;
        };
        if length < 3 {
            continue;
        }
        match open {
            None => open = Some((character, length)),
            Some((opening_character, opening_length))
                if character == opening_character
                    && length >= opening_length
                    && info.is_empty() =>
            {
                open = None;
            }
            Some(_) => {}
        }
    }
    open.is_some()
}

struct ProjectionBuilder {
    text: String,
    source_left_boundaries: Vec<usize>,
    source_boundaries: Vec<usize>,
    runs: Vec<VisualRun>,
    atomic_ranges: Vec<AtomicVisualRange>,
    inline_wrapper_stack: Vec<(usize, usize)>,
    inline_wrappers: Vec<InlineWrapper>,
}

impl ProjectionBuilder {
    fn new() -> Self {
        Self {
            text: String::new(),
            source_left_boundaries: vec![0],
            source_boundaries: vec![0],
            runs: Vec::new(),
            atomic_ranges: Vec::new(),
            inline_wrapper_stack: Vec::new(),
            inline_wrappers: Vec::new(),
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
            self.source_left_boundaries.pop();
            self.source_boundaries.pop();
        }

        if trailing_fenced_code_block && self.text.ends_with('\n') {
            // pulldown-cmark includes the mandatory line terminator before a
            // closing fence in the code text. It terminates the final code
            // line, but is not itself an editable blank line.
            self.text.pop();
            self.source_left_boundaries.pop();
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
            let previous_boundary = self.source_boundaries.last().copied().unwrap_or_default();
            if line_break.end <= previous_boundary {
                // A bare CR can already be present in parser text rather than
                // a SoftBreak event. Do not project that source break twice.
                continue;
            }
            let visual_start = self.text.chars().count();
            self.text.push('\n');
            let boundary = line_break.end;
            self.source_left_boundaries.push(boundary);
            self.source_boundaries.push(boundary);
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
        self.atomic_ranges.retain_mut(|atomic| {
            atomic.visual.end = atomic.visual.end.min(length);
            atomic.visual.start < atomic.visual.end
        });
        VisualProjection {
            text: self.text,
            source_left_boundaries: self.source_left_boundaries,
            source_boundaries: self.source_boundaries,
            runs: self.runs,
            atomic_ranges: self.atomic_ranges,
            inline_wrappers: self.inline_wrappers,
        }
    }

    fn begin_inline_wrapper(&mut self, source_start: usize) {
        self.inline_wrapper_stack
            .push((self.text.chars().count(), source_start));
    }

    fn append_source_line_breaks_until(
        &mut self,
        source: &str,
        source_end: usize,
        style: VisualStyle,
    ) {
        let mut cursor = self
            .source_boundaries
            .last()
            .copied()
            .unwrap_or_default()
            .min(source_end);
        while cursor < source_end {
            let line_break_end = match source.as_bytes().get(cursor..) {
                Some([b'\r', b'\n', ..]) => Some(cursor + 2),
                Some([b'\r' | b'\n', ..]) => Some(cursor + 1),
                Some(_) => None,
                None => break,
            };
            if let Some(line_break_end) = line_break_end {
                let visual_start = self.text.chars().count();
                self.text.push('\n');
                self.source_left_boundaries.push(line_break_end);
                self.source_boundaries.push(line_break_end);
                push_run(&mut self.runs, visual_start..visual_start + 1, style);
                cursor = line_break_end;
            } else {
                cursor += source[cursor..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }

    fn end_inline_wrapper(&mut self, source_end: usize) {
        let Some((visual_start, source_start)) = self.inline_wrapper_stack.pop() else {
            return;
        };
        let visual_end = self.text.chars().count();
        if visual_start < visual_end && source_start < source_end {
            self.inline_wrappers.push(InlineWrapper {
                visual: visual_start..visual_end,
                source: source_start..source_end,
            });
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
                source,
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
                source,
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
            self.append_marker(source, &visual, line_start..line_start + consumed, style);
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

        self.append_mapped(
            source,
            &source[unmapped_start..],
            unmapped_start..source.len(),
            VisualStyle::default(),
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
                let boundary = source_start + consumed;
                self.source_left_boundaries.push(boundary);
                self.source_boundaries.push(boundary);
            }
            push_run(
                &mut self.runs,
                visual_start..self.text.chars().count(),
                style,
            );
            return;
        }
        self.append_transformed(source, rendered, range, style);
    }

    fn append_text_with_footnote_references(
        &mut self,
        source: &str,
        rendered: &str,
        range: Range<usize>,
        style: VisualStyle,
        references: &[Range<usize>],
        source_selection: Option<&Range<usize>>,
    ) {
        let first = references.partition_point(|reference| reference.end <= range.start);
        let count = references[first..].partition_point(|reference| reference.start < range.end);
        let overlapping = &references[first..first + count];
        if overlapping.is_empty() {
            self.append_mapped(source, rendered, range, style);
            return;
        }

        let mut cursor = range.start;
        for reference in overlapping {
            if cursor < reference.start {
                self.append_mapped(
                    source,
                    &source[cursor..reference.start],
                    cursor..reference.start,
                    style,
                );
            }
            if range.start <= reference.start {
                let label = &source[reference.start + 2..reference.end - 1];
                let mut reference_style = style;
                reference_style.link = true;
                reference_style.footnote = true;
                self.append_footnote_reference(
                    source,
                    label,
                    reference.clone(),
                    reference_style,
                    source_selection,
                );
            }
            cursor = cursor.max(reference.end);
        }
        if cursor < range.end {
            self.append_mapped(source, &source[cursor..range.end], cursor..range.end, style);
        }
    }

    fn append_revealed_inline_code(
        &mut self,
        source: &str,
        syntax_range: Range<usize>,
        content_range: Range<usize>,
        style: VisualStyle,
    ) {
        let mut marker_style = style;
        marker_style.marker = true;
        if syntax_range.start < content_range.start {
            self.append_mapped(
                source,
                &source[syntax_range.start..content_range.start],
                syntax_range.start..content_range.start,
                marker_style,
            );
        }
        if content_range.start < content_range.end {
            self.append_mapped(
                source,
                &source[content_range.clone()],
                content_range.clone(),
                style,
            );
        }
        if content_range.end < syntax_range.end {
            self.append_mapped(
                source,
                &source[content_range.end..syntax_range.end],
                content_range.end..syntax_range.end,
                marker_style,
            );
        }
    }

    fn append_footnote_reference(
        &mut self,
        source: &str,
        label: &str,
        source_range: Range<usize>,
        style: VisualStyle,
        source_selection: Option<&Range<usize>>,
    ) {
        if source_selection
            .is_some_and(|selection| selection_reveals_hidden_source(selection, &source_range))
        {
            let mut revealed_style = style;
            revealed_style.marker = true;
            self.append_mapped(
                source,
                &source[source_range.clone()],
                source_range,
                revealed_style,
            );
        } else {
            self.append_transformed(source, &format!("〔{label}〕"), source_range, style);
        }
    }

    fn append_html(
        &mut self,
        source: &str,
        source_range: Range<usize>,
        style: VisualStyle,
        source_selection: Option<&Range<usize>>,
    ) {
        let tags = html_tag_ranges(&source[source_range.clone()], source_range.start);
        let mut visible_cursor = source_range.start;
        let mut has_visible_text = false;
        for tag in &tags {
            has_visible_text |= !source[visible_cursor..tag.start].trim().is_empty();
            visible_cursor = tag.end;
        }
        has_visible_text |= !source[visible_cursor..source_range.end].trim().is_empty();
        let tag_only_block =
            !has_visible_text && source_range.start == 0 && source_range.end == source.len();
        if tag_only_block && source_selection.is_none() {
            self.append_marker(source, "◇ HTML", source_range, style);
            return;
        }
        let reveal_tag_only_html = source_selection.is_some() && tag_only_block;

        let mut cursor = source_range.start;
        for tag in tags {
            if cursor < tag.start {
                self.append_mapped(source, &source[cursor..tag.start], cursor..tag.start, style);
            }
            if reveal_tag_only_html
                || source_selection
                    .is_some_and(|selection| selection_reveals_hidden_source(selection, &tag))
            {
                let mut marker = style;
                marker.marker = true;
                self.append_mapped(source, &source[tag.clone()], tag.clone(), marker);
            } else {
                self.set_current_boundary(tag.end);
            }
            cursor = tag.end;
        }
        if cursor < source_range.end {
            self.append_mapped(
                source,
                &source[cursor..source_range.end],
                cursor..source_range.end,
                style,
            );
        }
    }

    fn append_transformed(
        &mut self,
        source: &str,
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
        let source_boundaries = source[source_range.clone()]
            .char_indices()
            .map(|(index, _)| source_range.start + index)
            .chain(std::iter::once(source_range.end))
            .collect::<Vec<_>>();
        for index in 1..=rendered_chars {
            let source_index = index * source_boundaries.len().saturating_sub(1) / rendered_chars;
            let boundary = source_boundaries[source_index];
            self.source_left_boundaries.push(boundary);
            self.source_boundaries.push(boundary);
        }
        let visual_end = self.text.chars().count();
        if !source_range.is_empty() {
            self.atomic_ranges.push(AtomicVisualRange {
                visual: visual_start..visual_end,
                source: source_range,
            });
        }
        push_run(&mut self.runs, visual_start..visual_end, style);
    }

    fn append_marker(
        &mut self,
        source: &str,
        rendered: &str,
        source_range: Range<usize>,
        mut style: VisualStyle,
    ) {
        style.marker = true;
        self.append_transformed(source, rendered, source_range, style);
    }

    fn append_virtual(&mut self, rendered: &str, source_byte: usize, style: VisualStyle) {
        if rendered.is_empty() {
            return;
        }
        self.set_current_boundary(source_byte);
        let visual_start = self.text.chars().count();
        self.text.push_str(rendered);
        self.source_left_boundaries
            .extend(std::iter::repeat_n(source_byte, rendered.chars().count()));
        self.source_boundaries
            .extend(std::iter::repeat_n(source_byte, rendered.chars().count()));
        push_run(
            &mut self.runs,
            visual_start..self.text.chars().count(),
            style,
        );
    }

    fn ensure_line_break(&mut self, source_byte: usize, style: VisualStyle) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            let source_byte =
                source_byte.max(self.source_boundaries.last().copied().unwrap_or_default());
            let start = self.text.chars().count();
            self.text.push('\n');
            self.source_left_boundaries.push(source_byte);
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

fn inline_code_content_range(
    source: &str,
    rendered: &str,
    syntax_range: Range<usize>,
) -> Range<usize> {
    let fragment = &source[syntax_range.clone()];
    fragment
        .find(rendered)
        .map_or(syntax_range.clone(), |start| {
            syntax_range.start + start..syntax_range.start + start + rendered.len()
        })
}

fn standalone_footnote_references(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut references = Vec::new();
    let mut cursor = 0usize;
    while cursor + 3 < bytes.len() {
        let Some(relative_start) = text[cursor..].find("[^") else {
            break;
        };
        let start = cursor + relative_start;
        let escaped = text[..start]
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'\\')
            .count()
            % 2
            == 1;
        let label_start = start + 2;
        let Some(relative_end) = text[label_start..].find(']') else {
            break;
        };
        let end = label_start + relative_end + 1;
        let label = &text[label_start..end - 1];
        if !escaped && !label.is_empty() && label.len() <= 128 && !label.contains(['\r', '\n', '['])
        {
            references.push(start..end);
        }
        cursor = end;
    }
    references
}

fn inline_code_delimited_content_range(
    source: &str,
    syntax_range: Range<usize>,
) -> Option<Range<usize>> {
    let fragment = source.get(syntax_range.clone())?;
    let opening = fragment.bytes().take_while(|byte| *byte == b'`').count();
    let closing = fragment
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'`')
        .count();
    if opening == 0 || closing < opening || fragment.len() < opening.saturating_mul(2) {
        return None;
    }
    Some(syntax_range.start + opening..syntax_range.end - opening)
}

fn selection_reveals_inline_code(
    source_selection: &Range<usize>,
    content_range: &Range<usize>,
) -> bool {
    if source_selection.is_empty() {
        return content_range.start <= source_selection.start
            && source_selection.start <= content_range.end;
    }
    source_selection.start <= content_range.end && source_selection.end >= content_range.start
}

fn selection_reveals_hidden_source(
    source_selection: &Range<usize>,
    source_range: &Range<usize>,
) -> bool {
    if source_selection.is_empty() {
        return source_range.start < source_selection.start
            && source_selection.start < source_range.end;
    }
    source_selection.start <= source_range.end && source_selection.end >= source_range.start
}

fn visual_change_intersects(change: &Range<usize>, atomic: &Range<usize>) -> bool {
    if change.is_empty() {
        atomic.start < change.start && change.start < atomic.end
    } else {
        change.start < atomic.end && change.end > atomic.start
    }
}

fn html_tag_ranges(fragment: &str, source_offset: usize) -> Vec<Range<usize>> {
    let bytes = fragment.as_bytes();
    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if bytes[cursor] != b'<' || !looks_like_html_tag_start(bytes.get(cursor + 1).copied()) {
            cursor += fragment[cursor..].chars().next().map_or(1, char::len_utf8);
            continue;
        }

        let start = cursor;
        if fragment[start..].starts_with("<!--") {
            cursor = fragment[start + 4..]
                .find("-->")
                .map_or(bytes.len(), |end| start + 4 + end + 3);
        } else {
            cursor += 1;
            let mut quote = None;
            while cursor < bytes.len() {
                match bytes[cursor] {
                    b'\'' | b'"' if quote.is_none() => quote = Some(bytes[cursor]),
                    byte if quote == Some(byte) => quote = None,
                    b'>' if quote.is_none() => {
                        cursor += 1;
                        break;
                    }
                    _ => {}
                }
                cursor += 1;
            }
        }
        ranges.push(source_offset + start..source_offset + cursor);
    }
    ranges
}

fn looks_like_html_tag_start(character: Option<u8>) -> bool {
    character.is_some_and(|character| {
        character.is_ascii_alphabetic() || matches!(character, b'/' | b'!' | b'?')
    })
}

fn table_separator_range(source: &str, cell_start: usize) -> Option<Range<usize>> {
    let line_start = source[..cell_start]
        .rfind(['\n', '\r'])
        .map_or(0, |index| index + 1);
    source[line_start..cell_start]
        .match_indices('|')
        .rev()
        .find_map(|(relative, _)| {
            let pipe = line_start + relative;
            let escaped = source[line_start..pipe]
                .bytes()
                .rev()
                .take_while(|byte| *byte == b'\\')
                .count()
                % 2
                == 1;
            (!escaped).then_some(pipe..pipe + 1)
        })
}

fn fenced_code_content_start(source: &str, block_start: usize) -> usize {
    source[block_start..]
        .find('\n')
        .map_or(source.len(), |offset| block_start + offset + 1)
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

fn text_change_anchored_at_selection(
    before: &str,
    after: &str,
    after_selection: &Range<usize>,
) -> Option<TextChange> {
    let fallback = text_change(before, after)?;
    if !after_selection.is_empty() {
        return Some(fallback);
    }
    let old_length = fallback.old.len();
    let new_length = fallback.new.len();
    let new_end = after_selection.end;
    let Some(new_start) = new_end.checked_sub(new_length) else {
        return Some(fallback);
    };
    let old_start = new_start;
    let Some(old_end) = old_start.checked_add(old_length) else {
        return Some(fallback);
    };
    let before_length = before.chars().count();
    let after_length = after.chars().count();
    if old_end > before_length || new_end > after_length {
        return Some(fallback);
    }
    let candidate = TextChange {
        old: old_start..old_end,
        new: new_start..new_end,
    };
    if text_change_ranges_match(before, after, &candidate) {
        Some(candidate)
    } else {
        Some(fallback)
    }
}

fn text_change_ranges_match(before: &str, after: &str, change: &TextChange) -> bool {
    let before_old_start = char_to_byte(before, change.old.start);
    let before_old_end = char_to_byte(before, change.old.end);
    let after_new_start = char_to_byte(after, change.new.start);
    let after_new_end = char_to_byte(after, change.new.end);
    before[..before_old_start] == after[..after_new_start]
        && before[before_old_end..] == after[after_new_end..]
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
        if marker[cursor..].starts_with("[ ]")
            || marker[cursor..].starts_with("[x]")
            || marker[cursor..].starts_with("[X]")
        {
            let checked =
                marker[cursor..].starts_with("[x]") || marker[cursor..].starts_with("[X]");
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

fn trailing_line_break_boundary(source: &str, range: &Range<usize>) -> usize {
    // Container ranges can include indentation after their final newline.
    // Anchor the visual break to that newline, leaving trailing spaces for
    // append_trailing_editable_whitespace instead of mapping past them.
    let fragment = &source[range.clone()];
    let content_end = fragment.trim_end_matches([' ', '\t', '\n', '\r']).len();
    let Some(relative) = fragment[content_end..].find(['\n', '\r']) else {
        return range.end;
    };
    let line_break = range.start + content_end + relative;
    if source.as_bytes().get(line_break) == Some(&b'\r')
        && source.as_bytes().get(line_break + 1) == Some(&b'\n')
    {
        (line_break + 2).min(range.end)
    } else {
        (line_break + 1).min(range.end)
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

    fn type_visual_frame(source: &str, selection: Range<usize>, typed: &str) -> VisualSourceEdit {
        let projection =
            VisualProjection::from_markdown_with_selection(source, Some(selection.clone()));
        let visual_selection = projection.visual_char_range(source, selection);
        let mut edited = projection.text().to_owned();
        let start = char_to_byte(&edited, visual_selection.start);
        let end = char_to_byte(&edited, visual_selection.end);
        edited.replace_range(start..end, typed);
        let cursor = visual_selection.start + typed.chars().count();
        projection
            .apply_edit(source, &edited, cursor..cursor)
            .expect("typed frame should update the source")
    }

    fn safe_inline_text(length: Range<usize>) -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just('a'),
                Just('b'),
                Just('Z'),
                Just('0'),
                Just('中'),
                Just('文'),
                Just('🙂'),
            ],
            length,
        )
        .prop_map(|characters| characters.into_iter().collect())
    }

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
    fn staged_atx_heading_typing_keeps_the_marker_at_the_start() {
        let mut update = type_visual_frame("", 0..0, "#");
        assert_eq!(update.source, "#");
        update = type_visual_frame(&update.source, update.selection, " ");
        assert_eq!(update.source, "# ");
        update = type_visual_frame(&update.source, update.selection, "ATX 标题");
        assert_eq!(update.source, "# ATX 标题");

        update = type_visual_frame(&update.source, update.selection, "\n");
        update.selection = complete_visual_enter(&mut update.source, update.selection, false);
        assert_eq!(update.source, "# ATX 标题\n");
        update = type_visual_frame(&update.source, update.selection, "Setext 标题");
        assert_eq!(update.source, "# ATX 标题\nSetext 标题");
    }

    #[test]
    fn reveals_inline_code_delimiters_only_while_the_caret_is_inside() {
        let source = "a `code` z";
        let content_start = source.find("code").unwrap();
        let content_end = content_start + "code".len();

        let collapsed = VisualProjection::from_markdown(source);
        assert_eq!(collapsed.text(), "a code z");
        assert!(collapsed.runs.iter().any(|run| run.style.code));

        for cursor in content_start..=content_end {
            let revealed =
                VisualProjection::from_markdown_with_selection(source, Some(cursor..cursor));
            assert_eq!(revealed.text(), source, "cursor: {cursor}");
            assert!(revealed.runs.iter().any(|run| run.style.code));
            assert!(revealed.runs.iter().any(|run| run.style.marker));
        }

        for cursor in [content_start - 1, content_end + 1] {
            assert_eq!(
                VisualProjection::from_markdown_with_selection(source, Some(cursor..cursor)).text(),
                collapsed.text(),
                "cursor: {cursor}"
            );
        }
    }

    #[test]
    fn renders_footnote_references_even_when_the_definition_is_in_another_block() {
        let source = "脚注引用[^1] 与 [^说明]";
        let projection = VisualProjection::from_markdown(source);

        assert_eq!(projection.text(), "脚注引用〔1〕 与 〔说明〕");
        let footnotes = projection
            .runs_for(projection.text())
            .into_iter()
            .filter(|run| run.style.footnote)
            .collect::<Vec<_>>();
        assert_eq!(footnotes.len(), 2);
        assert!(footnotes.iter().all(|run| run.style.link));

        let escaped = VisualProjection::from_markdown(r"转义 \[^1]");
        assert_eq!(escaped.text(), "转义 [^1]");
        assert!(
            !escaped
                .runs_for(escaped.text())
                .iter()
                .any(|run| run.style.footnote)
        );

        let code = VisualProjection::from_markdown("```text\n[^1]\n```");
        assert_eq!(code.text(), "[^1]");
        assert!(
            !code
                .runs_for(code.text())
                .iter()
                .any(|run| run.style.footnote)
        );
    }

    #[test]
    fn footnote_references_reveal_for_exact_edits_and_fall_back_to_atomic_replacement() {
        let source = "脚注[^12]";
        let collapsed = VisualProjection::from_markdown(source);
        assert_eq!(collapsed.text(), "脚注〔12〕");

        let label_start = source[..source.find("12").unwrap()].chars().count();
        let revealed =
            VisualProjection::from_markdown_with_selection(source, Some(label_start..label_start));
        assert_eq!(revealed.text(), source);
        let edited = revealed.text().replacen('1', "9", 1);
        let update = revealed
            .apply_edit(source, &edited, label_start + 1..label_start + 1)
            .unwrap();
        assert_eq!(update.source, "脚注[^92]");

        let one = collapsed.text().find('1').unwrap();
        let visual_one = collapsed.text()[..one].chars().count();
        let mut edited = collapsed.text().to_owned();
        edited.replace_range(one..one + 1, "替换");
        let update = collapsed
            .apply_edit(source, &edited, visual_one + 2..visual_one + 2)
            .unwrap();
        assert_eq!(update.source, "脚注替换");
        assert_eq!(update.selection, 4..4);
    }

    #[test]
    fn html_projection_preserves_visible_text_and_quoted_angle_brackets() {
        let source = "before <span title=\"a > b\">重点</span> after";
        let projection = VisualProjection::from_markdown(source);
        assert_eq!(projection.text(), "before 重点 after");
        assert!(!projection.text().contains("title"));

        let content_byte = source.find("重点").unwrap();
        let content_start = source[..content_byte].chars().count();
        let update = type_visual_frame(source, content_start..content_start + 2, "关键");
        assert_eq!(
            update.source,
            "before <span title=\"a > b\">关键</span> after"
        );

        let tag_cursor = source[..source.find("a > b").unwrap() + 2].chars().count();
        let revealed =
            VisualProjection::from_markdown_with_selection(source, Some(tag_cursor..tag_cursor));
        assert_eq!(revealed.text(), "before <span title=\"a > b\">重点 after");
        assert!(revealed.runs.iter().any(|run| run.style.marker));

        let block = "<div data-value=\"a > b\">文字</div>";
        assert_eq!(VisualProjection::from_markdown(block).text(), "文字");

        let tag_only = "<img alt=\"diagram\" src=\"image.png\">";
        assert_eq!(VisualProjection::from_markdown(tag_only).text(), "◇ HTML");
        assert_eq!(
            VisualProjection::from_markdown_with_selection(tag_only, Some(0..0)).text(),
            tag_only
        );
    }

    #[test]
    fn links_and_images_reveal_editable_destinations_only_while_active() {
        let link = "before [文档](notes/old.md) after";
        let label_byte = link.find("文档").unwrap();
        let cursor = link[..label_byte].chars().count() + 1;
        let projection = VisualProjection::from_markdown_with_selection(link, Some(cursor..cursor));
        assert_eq!(projection.text(), link);
        let mut edited = projection.text().replace("old.md", "new.md");
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(link, &edited, cursor..cursor)
            .expect("link destination should be directly editable");
        assert_eq!(update.source, "before [文档](notes/new.md) after");

        let image = "![截图](assets/old.png)";
        let projection = VisualProjection::from_markdown_with_selection(image, Some(2..2));
        assert_eq!(projection.text(), image);
        edited = projection.text().replace("old.png", "new.png");
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(image, &edited, cursor..cursor)
            .expect("image destination should be directly editable");
        assert_eq!(update.source, "![截图](assets/new.png)");

        assert_eq!(
            VisualProjection::from_markdown(link).text(),
            "before 文档 after"
        );
        assert_eq!(VisualProjection::from_markdown(image).text(), "▧ 截图");
    }

    #[test]
    fn table_separator_edits_modify_the_backing_pipe_instead_of_being_discarded() {
        let source = "| a | b |\n| --- | --- |\n| 1 | 2 |";
        let projection = VisualProjection::from_markdown(source);
        let runs = projection.runs_for(projection.text());
        assert!(runs.iter().any(|run| run.style.table_header));
        assert!(
            runs.iter()
                .any(|run| run.style.table && !run.style.table_header)
        );
        let separator_byte = projection.text().find('│').unwrap();
        let separator = projection.text()[..separator_byte].chars().count();
        let mut edited = projection.text().to_owned();
        edited.remove(separator_byte);
        let update = projection
            .apply_edit(source, &edited, separator..separator)
            .unwrap();

        assert_eq!(
            update.source.matches('|').count(),
            source.matches('|').count() - 1
        );
    }

    #[test]
    fn collapsed_inline_code_keeps_edits_on_the_caret_side() {
        let source = "`abc`";

        let before = VisualProjection::from_markdown_with_selection(source, Some(0..0));
        assert_eq!(before.text(), "abc");
        assert_eq!(before.source_char_range(source, 0..0), 0..0);
        let inserted_before = before.apply_edit(source, "Xabc", 1..1).unwrap();
        assert_eq!(inserted_before.source, "X`abc`");

        let after = VisualProjection::from_markdown_with_selection(source, Some(5..5));
        assert_eq!(after.text(), "abc");
        assert_eq!(after.source_char_range(source, 3..3), 5..5);
        let inserted_after = after.apply_edit(source, "abcX", 4..4).unwrap();
        assert_eq!(inserted_after.source, "`abc`X");
        assert_eq!(inserted_after.selection, 6..6);
    }

    #[test]
    fn a_space_typed_after_inline_code_stays_outside_the_code_style() {
        let source = "`无法访问是`";
        let cursor = source.chars().count();
        let projection =
            VisualProjection::from_markdown_with_selection(source, Some(cursor..cursor));
        let mut edited = projection.text().to_owned();
        edited.push(' ');
        let visual_cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, &edited, visual_cursor..visual_cursor)
            .unwrap();

        assert_eq!(update.source, "`无法访问是` ");
        let reparsed = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert_eq!(reparsed.text(), "无法访问是 ");
        let trailing = reparsed
            .runs
            .iter()
            .find(|run| run.range.contains(&(reparsed.text().chars().count() - 1)))
            .unwrap();
        assert!(!trailing.style.code);
        assert!(!trailing.style.marker);
    }

    #[test]
    fn arrow_keys_stop_on_the_hidden_inline_code_delimiters() {
        let source = "`ab`";
        assert_eq!(
            move_across_hidden_inline_code_boundary(source, 4..4, true, false),
            Some(3..3)
        );
        assert_eq!(
            move_across_hidden_inline_code_boundary(source, 0..0, false, true),
            Some(1..1)
        );
        assert_eq!(
            move_across_hidden_inline_code_boundary(source, 3..3, true, false),
            None
        );
        assert_eq!(
            move_across_hidden_inline_code_boundary(source, 1..1, false, true),
            None
        );

        let multi = "``a ` b``";
        let after = multi.chars().count();
        assert_eq!(
            move_across_hidden_inline_code_boundary(multi, after..after, true, false),
            Some(after - 2..after - 2)
        );
        assert_eq!(
            move_across_hidden_inline_code_boundary(multi, 0..0, false, true),
            Some(2..2)
        );
    }

    #[test]
    fn edits_revealed_inline_code_without_losing_its_delimiters() {
        let source = "a `code` z";
        let projection = VisualProjection::from_markdown_with_selection(source, Some(4..4));
        let edited = "a `coder` z";
        let cursor = edited.find('r').unwrap() + 1;
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();

        assert_eq!(update.source, edited);
        assert_eq!(update.selection, cursor..cursor);
        assert_eq!(VisualProjection::from_markdown(edited).text(), "a coder z");
    }

    #[test]
    fn reveals_only_the_selected_unicode_inline_code_span() {
        let source = "前 `代码` 与 `second` 后";
        let second_byte = source.find("second").unwrap();
        let second_start = source[..second_byte].chars().count();
        let projection = VisualProjection::from_markdown_with_selection(
            source,
            Some(second_start + 2..second_start + 2),
        );

        assert_eq!(projection.text(), "前 代码 与 `second` 后");
        assert_eq!(projection.text().matches('`').count(), 2);
    }

    #[test]
    fn reveals_multi_backtick_delimiters_around_embedded_backticks() {
        let source = "before ``a ` b`` after";
        let content_byte = source.find("a ` b").unwrap();
        let content_start = source[..content_byte].chars().count();
        let projection = VisualProjection::from_markdown_with_selection(
            source,
            Some(content_start + 2..content_start + 2),
        );

        assert_eq!(projection.text(), source);
        assert_eq!(
            VisualProjection::from_markdown(source).text(),
            "before a ` b after"
        );
    }

    #[test]
    fn keeps_incomplete_backtick_sequences_visible_for_direct_typing() {
        assert_eq!(VisualProjection::from_markdown("`").text(), "`");
        assert_eq!(VisualProjection::from_markdown("``").text(), "``");
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
    fn deleting_a_complete_formatted_run_removes_both_hidden_delimiters() {
        for (source, edited, expected) in [
            ("**a**b", "b", "b"),
            ("*a*b", "b", "b"),
            ("~~a~~b", "b", "b"),
            ("**a**b", "", ""),
            ("a**b**", "", ""),
        ] {
            let projection = VisualProjection::from_markdown(source);
            let update = projection.apply_edit(source, edited, 0..0).unwrap();
            assert_eq!(update.source, expected, "source: {source:?}");
        }

        let source = "**ab**c";
        let projection = VisualProjection::from_markdown(source);
        let update = projection.apply_edit(source, "ac", 1..1).unwrap();
        assert_eq!(update.source, "**a**c");
    }

    #[test]
    fn empty_inline_format_pairs_are_editable_without_block_syntax_conflicts() {
        for (source, cursor, typed, expected) in [
            ("****", 2, "bold", "**bold**"),
            ("~~~~", 2, "gone", "~~gone~~"),
            ("**", 1, "em", "*em*"),
            ("``", 1, "code", "`code`"),
        ] {
            let projection =
                VisualProjection::from_markdown_with_selection(source, Some(cursor..cursor));
            assert_eq!(projection.text(), "", "source: {source:?}");
            let end = typed.chars().count();
            let update = projection.apply_edit(source, typed, end..end).unwrap();
            assert_eq!(update.source, expected, "source: {source:?}");
        }
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
            ("- [X] 完成", "- [X] 完成\n- [ ] ", "☑ 完成\n☐ "),
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
    fn container_internal_blank_lines_remain_visible_and_individually_editable() {
        for source in ["- a\n\n\n- b", "> a\n>\n>\n> b"] {
            let projection = VisualProjection::from_markdown(source);
            assert_eq!(
                projection.text().matches('\n').count(),
                source.matches('\n').count(),
                "source: {source:?}, visual: {:?}",
                projection.text()
            );

            let middle_newline = projection
                .text()
                .match_indices('\n')
                .nth(1)
                .map(|(byte, _)| projection.text()[..byte].chars().count())
                .unwrap();
            let mut edited = projection.text().to_owned();
            edited.remove(char_to_byte(&edited, middle_newline));
            let update = projection
                .apply_edit(source, &edited, middle_newline..middle_newline)
                .unwrap();
            assert_eq!(
                update.source.matches('\n').count(),
                source.matches('\n').count() - 1,
                "source: {source:?}, updated: {:?}",
                update.source
            );
        }
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
    fn clicked_inter_block_blank_line_projects_to_its_visual_row() {
        let source = "前置 `无法访问是`  后置\n\n\n\n";
        let cursor = "前置 `无法访问是`  后置\n\n".chars().count();
        let projection =
            VisualProjection::from_markdown_with_selection(source, Some(cursor..cursor));

        assert_eq!(projection.text(), "前置 无法访问是  后置\n\n\n\n");
        let visual_cursor = "前置 无法访问是  后置\n\n".chars().count();
        assert_eq!(
            projection.visual_char_range(source, cursor..cursor),
            visual_cursor..visual_cursor
        );
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
    fn unfinished_fence_has_an_editable_anchor_and_enter_completes_the_block() {
        let source = "```";
        let projection = VisualProjection::from_markdown(source);
        assert_eq!(projection.text(), "");
        assert_eq!(projection.source_boundaries, vec![source.len()]);

        let mut update = projection.apply_edit(source, "\n", 1..1).unwrap();
        assert_eq!(update.source, "```\n");
        assert_eq!(update.selection, 4..4);
        update.selection =
            complete_fenced_code_on_enter(&mut update.source, update.selection.clone()).unwrap();
        assert_eq!(update.source, "```\n\n```");
        assert_eq!(update.selection, 4..4);

        let completed = VisualProjection::from_markdown(&update.source);
        assert_eq!(completed.text(), "");
        let code = completed.apply_edit(&update.source, "value", 5..5).unwrap();
        assert_eq!(code.source, "```\nvalue\n```");
    }

    #[test]
    fn third_fence_marker_immediately_places_the_cursor_in_the_code_body() {
        for (before, expected, cursor) in [
            ("```", "```\n\n```", 4),
            ("~~~", "~~~\n\n~~~", 4),
            ("  ```", "  ```\n\n  ```", 6),
            ("paragraph\n\n```", "paragraph\n\n```\n\n```", 15),
        ] {
            let mut source = before.to_owned();
            let end = source.chars().count();
            let selection = complete_bare_fenced_code_after_typing(&mut source, end..end).unwrap();
            assert_eq!(source, expected);
            assert_eq!(selection, cursor..cursor);
        }

        let mut language = "```rust".to_owned();
        let end = language.chars().count();
        assert!(
            complete_bare_fenced_code_after_typing(&mut language, end..end).is_none(),
            "a fence and info string delivered in one input event remains editable until Enter"
        );
        let mut nested = "```\ncode\n```".to_owned();
        let end = nested.chars().count();
        assert!(complete_bare_fenced_code_after_typing(&mut nested, end..end).is_none());

        let mut indented = "    ```".to_owned();
        let end = indented.chars().count();
        assert!(complete_bare_fenced_code_after_typing(&mut indented, end..end).is_none());
        assert_eq!(indented, "    ```");

        let mut stale_cursor = "```".to_owned();
        assert_eq!(
            complete_bare_fenced_code_after_typing(&mut stale_cursor, 2..2),
            Some(4..4)
        );
        assert_eq!(stale_cursor, "```\n\n```");
    }

    #[test]
    fn an_explicit_closer_consumes_the_hidden_paired_closer_and_exits() {
        for (before, expected) in [
            ("```\ncode\n```\n```", "```\ncode\n```\n\n"),
            ("~~~text\n中文\n~~~\n~~~", "~~~text\n中文\n~~~\n\n"),
        ] {
            let mut source = before.to_owned();
            let typed_end = source[..source.rfind('\n').unwrap()].chars().count();
            let selection =
                consume_paired_fenced_code_closer(&mut source, typed_end..typed_end).unwrap();
            assert_eq!(source, expected);
            assert_eq!(
                selection,
                expected.chars().count()..expected.chars().count()
            );
        }

        let mut longer_outer = "````\n```\n````".to_owned();
        let cursor = "````\n```".chars().count();
        assert!(consume_paired_fenced_code_closer(&mut longer_outer, cursor..cursor).is_none());

        let mut stale_cursor = "```\ncode\n```\n```".to_owned();
        assert_eq!(
            consume_paired_fenced_code_closer(&mut stale_cursor, 2..3),
            Some(14..14)
        );
        assert_eq!(stale_cursor, "```\ncode\n```\n\n");
    }

    #[test]
    fn arrow_exit_addresses_a_normal_paragraph_after_the_closing_fence() {
        for (before, expected, cursor) in [
            ("```\ncode\n```", "```\ncode\n```\n\n", 14),
            ("```rust\ncode\n```\n", "```rust\ncode\n```\n\n", 18),
            ("~~~\ncode\n~~~\n\n\n", "~~~\ncode\n~~~\n\n\n", 14),
        ] {
            let mut source = before.to_owned();
            let selection = paragraph_after_fenced_code(&mut source).unwrap();
            assert_eq!(source, expected);
            assert_eq!(selection, cursor..cursor);
        }
    }

    #[test]
    fn exposes_the_fenced_language_without_exposing_the_markers() {
        assert_eq!(
            fenced_code_language("```rust\nfn main() {}\n```"),
            Some("rust")
        );
        assert_eq!(
            fenced_code_language("~~~ text linenos\nvalue\n~~~"),
            Some("text linenos")
        );
        assert_eq!(fenced_code_language("```\nvalue\n```"), None);
    }

    #[test]
    fn exposes_only_complete_fenced_code_content_for_copying() {
        assert_eq!(
            fenced_code_content("```rust\nfn main() {}\nlet n = 1;\n```"),
            Some("fn main() {}\nlet n = 1;")
        );
        assert_eq!(fenced_code_content("~~~\r\n中文\r\n~~~\r\n"), Some("中文"));
        assert_eq!(fenced_code_content("```\n```"), Some(""));
        assert_eq!(fenced_code_content("```\n\n```"), Some(""));
        assert_eq!(fenced_code_content("```\nunclosed"), None);
    }

    #[test]
    fn completing_a_fence_preserves_language_length_and_indentation() {
        for (before, after) in [
            ("```rust\n", "```rust\n\n```"),
            ("  ~~~~text\n", "  ~~~~text\n\n  ~~~~"),
        ] {
            let mut source = before.to_owned();
            let cursor = source.chars().count();
            let selection = complete_fenced_code_on_enter(&mut source, cursor..cursor).unwrap();
            assert_eq!(source, after);
            assert_eq!(selection, cursor..cursor);
        }

        let mut plain = "ordinary\n".to_owned();
        let cursor = plain.chars().count();
        assert!(complete_fenced_code_on_enter(&mut plain, cursor..cursor).is_none());
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
        fn visible_inline_format_edits_round_trip_without_losing_hidden_syntax(
            prefix in safe_inline_text(0..8),
            content in safe_inline_text(1..20),
            suffix in safe_inline_text(0..8),
            delimiter_index in 0usize..4,
            raw_start in any::<usize>(),
            raw_end in any::<usize>(),
            delete in any::<bool>(),
        ) {
            let delimiter = ["*", "**", "~~", "`"][delimiter_index];
            let source = format!("{prefix}{delimiter}{content}{delimiter}{suffix}");
            let projection = VisualProjection::from_markdown(&source);
            let expected_visual = format!("{prefix}{content}{suffix}");
            prop_assume!(projection.text() == expected_visual);

            let content_len = content.chars().count();
            let mut local_start = raw_start % (content_len + 1);
            let mut local_end = raw_end % (content_len + 1);
            if local_start > local_end {
                std::mem::swap(&mut local_start, &mut local_end);
            }
            if local_start == local_end {
                local_end = (local_end + 1).min(content_len);
                if local_start == local_end {
                    local_start = local_start.saturating_sub(1);
                }
            }
            let visual_start = prefix.chars().count() + local_start;
            let visual_end = prefix.chars().count() + local_end;
            let replacement = if delete { "" } else { "中🙂" };
            let mut edited = expected_visual;
            edited.replace_range(
                char_to_byte(&edited, visual_start)..char_to_byte(&edited, visual_end),
                replacement,
            );
            prop_assume!(edited != projection.text());
            let cursor = visual_start + replacement.chars().count();
            let update = projection
                .apply_edit(&source, &edited, cursor..cursor)
                .expect("a visible inline edit must map back to Markdown");
            let reparsed = VisualProjection::from_markdown(&update.source);
            prop_assert_eq!(
                reparsed.text(),
                edited,
                "updated source: {:?}",
                update.source
            );
            prop_assert!(update.selection.end <= update.source.chars().count());
        }

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
