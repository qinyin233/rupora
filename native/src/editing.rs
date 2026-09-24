use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownCommand {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Link,
    Heading(u8),
    Quote,
    BulletList,
    OrderedList,
    CodeBlock,
}

pub fn apply_markdown_command(
    text: &mut String,
    selection: Range<usize>,
    command: MarkdownCommand,
) -> Range<usize> {
    let selection = clamp_char_range(text, selection);
    match command {
        MarkdownCommand::Bold => toggle_emphasis(text, selection, "**"),
        MarkdownCommand::Italic => toggle_emphasis(text, selection, "*"),
        MarkdownCommand::Strikethrough => toggle_emphasis(text, selection, "~~"),
        MarkdownCommand::InlineCode => toggle_inline_code(text, selection),
        MarkdownCommand::Link => insert_link(text, selection),
        MarkdownCommand::Heading(level) => {
            transform_selected_lines(text, selection, LineCommand::Heading(level.clamp(1, 6)))
        }
        MarkdownCommand::Quote => transform_selected_lines(text, selection, LineCommand::Quote),
        MarkdownCommand::BulletList => {
            transform_selected_lines(text, selection, LineCommand::BulletList)
        }
        MarkdownCommand::OrderedList => {
            transform_selected_lines(text, selection, LineCommand::OrderedList)
        }
        MarkdownCommand::CodeBlock => toggle_code_block(text, selection),
    }
}

pub fn find_next(
    text: &str,
    query: &str,
    start_char: usize,
    match_case: bool,
) -> Option<Range<usize>> {
    if query.is_empty() {
        return None;
    }
    let start_byte = char_to_byte(text, start_char.min(text.chars().count()));
    find_from_byte(text, query, start_byte, match_case)
        .or_else(|| find_from_byte(text, query, 0, match_case))
}

pub fn find_previous(
    text: &str,
    query: &str,
    before_char: usize,
    match_case: bool,
) -> Option<Range<usize>> {
    if query.is_empty() {
        return None;
    }
    let before_byte = char_to_byte(text, before_char.min(text.chars().count()));
    let matches = collect_byte_matches(text, query, match_case);
    matches
        .iter()
        .rev()
        .find(|range| range.end <= before_byte)
        .or_else(|| matches.last())
        .map(|range| byte_range_to_char_range(text, range.clone()))
}

pub fn replace_range(text: &mut String, range: Range<usize>, replacement: &str) -> Range<usize> {
    let range = clamp_char_range(text, range);
    let start_byte = char_to_byte(text, range.start);
    let end_byte = char_to_byte(text, range.end);
    text.replace_range(start_byte..end_byte, replacement);
    let end = range.start + replacement.chars().count();
    end..end
}

pub fn replace_all(text: &mut String, query: &str, replacement: &str, match_case: bool) -> usize {
    let matches = collect_byte_matches(text, query, match_case);
    if matches.is_empty() {
        return 0;
    }
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for range in &matches {
        output.push_str(&text[cursor..range.start]);
        output.push_str(replacement);
        cursor = range.end;
    }
    output.push_str(&text[cursor..]);
    *text = output;
    matches.len()
}

pub fn selection_matches(text: &str, range: Range<usize>, query: &str, match_case: bool) -> bool {
    let range = clamp_char_range(text, range);
    let selected = &text[char_to_byte(text, range.start)..char_to_byte(text, range.end)];
    if match_case {
        selected == query
    } else {
        selected.eq_ignore_ascii_case(query)
    }
}

pub fn char_index_for_line(text: &str, one_based_line: usize) -> usize {
    if one_based_line <= 1 {
        return 0;
    }
    text.match_indices('\n')
        .nth(one_based_line - 2)
        .map_or_else(
            || text.chars().count(),
            |(byte_index, _)| text[..=byte_index].chars().count(),
        )
}

pub fn continue_markdown_line(text: &mut String, cursor: usize) -> Option<Range<usize>> {
    let cursor = cursor.min(text.chars().count());
    let cursor_byte = char_to_byte(text, cursor);
    let previous_newline = text[..cursor_byte].rfind('\n')?;
    let editor_indent = &text[previous_newline + 1..cursor_byte];
    if !editor_indent
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
    {
        return None;
    }

    let line_start = text[..previous_newline]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let previous_line = &text[line_start..previous_newline];
    let continuation = continuation_prefix(previous_line)?;
    // Markdown-looking lines inside fenced/indented code remain literal. Use
    // the parser so nested containers and unclosed fences follow the same rule.
    if pulldown_cmark::Parser::new_ext(text, crate::markdown::parser_options())
        .into_offset_iter()
        .any(|(event, range)| {
            matches!(
                event,
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(_))
            ) && range.contains(&previous_newline)
        })
    {
        return None;
    }
    let content = &previous_line[continuation.content_start..];
    let editor_indent_chars = editor_indent.chars().count();

    if content.trim().is_empty() {
        let removed_chars = previous_line[..continuation.source_prefix_len]
            .chars()
            .count();
        text.replace_range(previous_newline + 1..cursor_byte, "");
        text.replace_range(line_start..line_start + continuation.source_prefix_len, "");
        let next = cursor
            .saturating_sub(editor_indent_chars)
            .saturating_sub(removed_chars);
        return Some(next..next);
    }

    text.replace_range(previous_newline + 1..cursor_byte, &continuation.next_prefix);
    let next = cursor - editor_indent_chars + continuation.next_prefix.chars().count();
    Some(next..next)
}

pub fn indent_selected_lines(
    text: &mut String,
    selection: Range<usize>,
    outdent: bool,
) -> Range<usize> {
    let selection = clamp_char_range(text, selection);
    let collapsed = selection.is_empty();
    let start_byte = char_to_byte(text, selection.start);
    let end_byte = char_to_byte(text, selection.end);
    let Range {
        start: block_start,
        end: block_end,
    } = selected_line_bytes(text, start_byte..end_byte);
    let original = text[block_start..block_end].to_owned();
    let transformed = original
        .split('\n')
        .map(|line| {
            if outdent {
                line.strip_prefix('\t')
                    .or_else(|| line.strip_prefix("    "))
                    .or_else(|| line.strip_prefix("   "))
                    .or_else(|| line.strip_prefix("  "))
                    .or_else(|| line.strip_prefix(' '))
                    .unwrap_or(line)
                    .to_owned()
            } else {
                format!("    {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    text.replace_range(block_start..block_end, &transformed);
    let start = text[..block_start].chars().count();
    if collapsed {
        let mapped = map_prefix_edit_cursor(
            &original,
            &transformed,
            start_byte.saturating_sub(block_start),
        );
        start + mapped..start + mapped
    } else {
        start..start + transformed.chars().count()
    }
}

pub fn paste_url_as_markdown_link(
    text: &mut String,
    selection: Range<usize>,
    url: &str,
) -> Option<Range<usize>> {
    if !is_probable_url(url) || selection.is_empty() {
        return None;
    }
    let selection = clamp_char_range(text, selection);
    let start = char_to_byte(text, selection.start);
    let end = char_to_byte(text, selection.end);
    let label = escape_link_label(&text[start..end]);
    let destination = escape_link_destination(url);
    let replacement = format!("[{label}]({destination})");
    text.replace_range(start..end, &replacement);
    let cursor = selection.start + replacement.chars().count();
    Some(cursor..cursor)
}

pub fn insert_resource_link(
    text: &mut String,
    selection: Range<usize>,
    label: &str,
    destination: &str,
    image: bool,
) -> Range<usize> {
    let selection = clamp_char_range(text, selection);
    let label = escape_link_label(label);
    let destination = escape_link_destination(destination);
    let replacement = if image {
        format!("![{label}]({destination})")
    } else {
        format!("[{label}]({destination})")
    };
    replace_range(text, selection, &replacement)
}

pub fn apply_smart_pair(
    text: &mut String,
    selection: Range<usize>,
    typed: &str,
) -> Option<Range<usize>> {
    let selection = clamp_char_range(text, selection);
    if selection.is_empty() && matches!(typed, ")" | "]" | "}" | "\"" | "'") {
        let start = char_to_byte(text, selection.start);
        let end = start + typed.len();
        if text.get(start..end) == Some(typed) {
            let cursor = selection.start + 1;
            return Some(cursor..cursor);
        }
    }
    let (opening, closing) = match typed {
        "(" => ("(", ")"),
        "[" => ("[", "]"),
        "{" => ("{", "}"),
        "\"" => ("\"", "\""),
        "'" => ("'", "'"),
        _ => return None,
    };

    let start = char_to_byte(text, selection.start);
    let end = char_to_byte(text, selection.end);
    let selected = text[start..end].to_owned();
    let replacement = format!("{opening}{selected}{closing}");
    text.replace_range(start..end, &replacement);
    if selection.is_empty() {
        let cursor = selection.start + opening.chars().count();
        Some(cursor..cursor)
    } else {
        Some(selection.start + opening.chars().count()..selection.end + opening.chars().count())
    }
}

fn find_from_byte(
    text: &str,
    query: &str,
    start_byte: usize,
    match_case: bool,
) -> Option<Range<usize>> {
    if match_case {
        let relative = text[start_byte..].find(query)?;
        let byte_start = start_byte + relative;
        return Some(byte_range_to_char_range(
            text,
            byte_start..byte_start + query.len(),
        ));
    }

    text[start_byte..]
        .char_indices()
        .map(|(offset, _)| start_byte + offset)
        .find_map(|byte_start| {
            let end = byte_start.checked_add(query.len())?;
            let candidate = text.get(byte_start..end)?;
            candidate
                .eq_ignore_ascii_case(query)
                .then(|| byte_range_to_char_range(text, byte_start..end))
        })
}

fn collect_byte_matches(text: &str, query: &str, match_case: bool) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    if match_case {
        return text
            .match_indices(query)
            .map(|(start, matched)| start..start + matched.len())
            .collect();
    }

    let mut last_end = 0;
    text.char_indices()
        .map(|(index, _)| index)
        .filter_map(|start| {
            if start < last_end {
                return None;
            }
            let end = start.checked_add(query.len())?;
            let candidate = text.get(start..end)?;
            if candidate.eq_ignore_ascii_case(query) {
                last_end = end;
                Some(start..end)
            } else {
                None
            }
        })
        .collect()
}

fn toggle_emphasis(text: &mut String, selection: Range<usize>, marker: &str) -> Range<usize> {
    if selection.is_empty() {
        return toggle_wrap(text, selection, marker, marker);
    }
    if let Some(next) = toggle_paragraph_emphasis(text, selection.clone(), marker) {
        return next;
    }
    let selected = &text[char_to_byte(text, selection.start)..char_to_byte(text, selection.end)];
    let trimmed = selected.trim();
    if trimmed.is_empty() {
        return selection;
    }
    // Emphasis delimiters touching whitespace remain literal Markdown. Keep
    // that whitespace intact and select only the text receiving the format.
    let leading = selected.chars().take_while(|ch| ch.is_whitespace()).count();
    let start = selection.start + leading;
    let end = start + trimmed.chars().count();
    let start_byte = char_to_byte(text, start);
    let end_byte = char_to_byte(text, end);
    let surrounded = start_byte >= marker.len()
        && text.get(start_byte - marker.len()..start_byte) == Some(marker)
        && text.get(end_byte..end_byte + marker.len()) == Some(marker);
    if marker == "*"
        && surrounded
        && pulldown_cmark::Parser::new_ext(text, crate::markdown::parser_options())
            .into_offset_iter()
            .fold((false, false), |(strong, emphasis), (event, range)| {
                if range.start < start_byte && end_byte < range.end {
                    match event {
                        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Strong)
                            if range.start + 2 == start_byte && end_byte + 2 == range.end =>
                        {
                            (true, emphasis)
                        }
                        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Emphasis) => {
                            (strong, true)
                        }
                        _ => (strong, emphasis),
                    }
                } else {
                    (strong, emphasis)
                }
            })
            == (true, false)
    {
        // One star beside the selection may belong to a strong delimiter.
        // Adding italic must not remove half of the existing bold syntax.
        text.insert_str(end_byte, marker);
        text.insert_str(start_byte, marker);
        return start + marker.len()..end + marker.len();
    }
    toggle_wrap(text, start..end, marker, marker)
}

fn toggle_paragraph_emphasis(
    text: &mut String,
    selection: Range<usize>,
    marker: &str,
) -> Option<Range<usize>> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let selected = char_to_byte(text, selection.start)..char_to_byte(text, selection.end);
    if !text[selected.clone()].contains(['\r', '\n']) {
        if marker != "~~" {
            return None;
        }
        let body = &text[selected.clone()];
        let trimmed = body.trim();
        let start = selected.start + body.len() - body.trim_start().len();
        let end = start + trimmed.len();
        // Adding two tildes beside a literal tilde may open fenced code even
        // on one line. Validate that collision, but keep ordinary single-line
        // toggles (including headings) and removal of existing markers intact.
        if trimmed.is_empty()
            || (!trimmed.starts_with('~') && !text[..start].ends_with('~'))
            || (text[..start].ends_with(marker) && text[end..].starts_with(marker))
        {
            return None;
        }
    }
    struct Wrapper {
        range: Range<usize>,
        delimiter_len: usize,
        requested: bool,
    }
    let mut paragraphs = Vec::new();
    let mut paragraph = None;
    for (event, range) in
        Parser::new_ext(text, crate::markdown::parser_options()).into_offset_iter()
    {
        match event {
            Event::Start(Tag::Paragraph) => {
                paragraph = (range.start < selected.end && selected.start < range.end).then(|| {
                    paragraphs.push((range, Vec::<Wrapper>::new(), Vec::new()));
                    paragraphs.len() - 1
                });
            }
            Event::End(TagEnd::Paragraph) => paragraph = None,
            Event::Start(tag @ (Tag::Strong | Tag::Emphasis | Tag::Strikethrough)) => {
                if let Some(index) = paragraph {
                    let delimiter_len = match tag {
                        Tag::Strong => 2,
                        Tag::Strikethrough if text[range.clone()].starts_with("~~") => 2,
                        _ => 1,
                    };
                    let requested = matches!(
                        (marker, tag),
                        ("**", Tag::Strong) | ("*", Tag::Emphasis) | ("~~", Tag::Strikethrough)
                    );
                    paragraphs[index].1.push(Wrapper {
                        range,
                        delimiter_len,
                        requested,
                    });
                }
            }
            Event::Code(_)
            | Event::InlineHtml(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => {
                if let Some(index) = paragraph {
                    paragraphs[index].2.push(range);
                }
            }
            _ => {}
        }
    }
    if paragraphs.len() < 2 {
        // Wrapping across a block boundary can change how the next toggle
        // parses the document. Keep unsupported mixed-block selections intact.
        // Even one paragraph needs the validation below: adding `~~` before a
        // literal line-leading `~` can turn its soft line break into fenced code.
        let body = &text[selected.clone()];
        let start = selected.start + body.len() - body.trim_start().len();
        let end = start + body.trim().len();
        if !paragraphs
            .first()
            .is_some_and(|(paragraph, _, _)| paragraph.start <= start && end <= paragraph.end)
        {
            return Some(selection);
        }
    }
    let mut ranges = Vec::new();
    for (mut scope, wrappers, literals) in paragraphs {
        let body = &text[scope.clone()];
        scope.start += body.len() - body.trim_start().len();
        scope.end = scope.start + body.trim().len();
        if scope.start >= selected.end || selected.start >= scope.end {
            continue;
        }
        let mut range = scope.start.max(selected.start)..scope.end.min(selected.end);
        let body = &text[range.clone()];
        range.start += body.len() - body.trim_start().len();
        range.end = range.start + body.trim().len();
        if range.is_empty() {
            continue;
        }
        // A delimiter inserted inside code, math or an HTML tag is literal,
        // so it cannot apply emphasis or be recognized on the next toggle.
        // Keep that paragraph intact when the selection cuts such a token.
        if literals.iter().any(|literal| {
            (literal.start < range.start && range.start < literal.end)
                || (literal.start < range.end && range.end < literal.end)
        }) {
            continue;
        }
        // Work inside existing whole-paragraph emphasis. This preserves outer
        // formats and avoids interpreting one '*' from '**' as italic syntax.
        for wrapper in &wrappers {
            if wrapper.range.start <= range.start && range.end <= wrapper.range.end {
                if wrapper.requested
                    && wrapper.range.start + wrapper.delimiter_len == range.start
                    && wrapper.range.end.saturating_sub(wrapper.delimiter_len) == range.end
                {
                    // The selection is exactly the body of this format. Keep
                    // nested emphasis markers selected so the outer wrapper
                    // can be removed as one toggle.
                    break;
                }
                range.start = range.start.max(wrapper.range.start + wrapper.delimiter_len);
                range.end = range.end.min(wrapper.range.end - wrapper.delimiter_len);
            }
        }
        if !range.is_empty() {
            // A partial range may have a star from the outer wrapper before it
            // and a star from a nested wrapper after it. Only remove markers
            // when the range is the complete body of some parsed wrapper.
            let whole_wrapper_body = wrappers.iter().any(|wrapper| {
                wrapper.range.start + wrapper.delimiter_len == range.start
                    && wrapper.range.end.saturating_sub(wrapper.delimiter_len) == range.end
            });
            let formatted = whole_wrapper_body
                && wrappers.iter().any(|wrapper| {
                    wrapper.requested
                        && wrapper.range.start <= range.start
                        && range.end <= wrapper.range.end
                });
            ranges.push((range, formatted));
        }
    }
    if ranges.is_empty() {
        return Some(selection);
    }
    // Build once so formatting many paragraphs does not repeatedly shift or
    // scan the full document. Positions returned to the editor are characters.
    let mut output = String::with_capacity(text.len() + ranges.len() * marker.len() * 2);
    let mut read = 0;
    let mut written_chars = 0;
    let mut result = None;
    let mut changed_bodies = Vec::new();
    let mut added_wrappers = Vec::new();
    for (range, formatted) in ranges {
        let remove = formatted
            && range.start >= marker.len()
            && text.get(range.start - marker.len()..range.start) == Some(marker)
            && text.get(range.end..range.end + marker.len()) == Some(marker);
        let start = range.start - if remove { marker.len() } else { 0 };
        let end = range.end + if remove { marker.len() } else { 0 };
        let gap = &text[read..start];
        output.push_str(gap);
        written_chars += gap.chars().count();
        let wrapper_start = output.len();
        if !remove {
            output.push_str(marker);
            written_chars += marker.len();
        }
        let first = result.get_or_insert(written_chars..written_chars);
        let body_start = output.len();
        let body = &text[range];
        output.push_str(body);
        changed_bodies.push(body_start..output.len());
        written_chars += body.chars().count();
        first.end = written_chars;
        if !remove {
            output.push_str(marker);
            written_chars += marker.len();
            added_wrappers.push(wrapper_start..output.len());
        }
        read = end;
    }
    output.push_str(&text[read..]);
    // Escapes, partial links and delimiter-only paragraphs can turn new
    // markers into literal text or change the block type. Commit only an
    // emphasis edit that the parser can render and subsequently toggle.
    let mut output_paragraphs = Vec::new();
    let mut output_wrappers = std::collections::HashSet::new();
    for (event, range) in
        Parser::new_ext(&output, crate::markdown::parser_options()).into_offset_iter()
    {
        match event {
            Event::Start(Tag::Paragraph) => output_paragraphs.push(range),
            Event::Start(tag)
                if matches!(
                    (marker, &tag),
                    ("**", Tag::Strong) | ("*", Tag::Emphasis) | ("~~", Tag::Strikethrough)
                ) =>
            {
                output_wrappers.insert(emphasis_delimiter_key(&output, range, marker));
            }
            _ => {}
        }
    }
    if changed_bodies.iter().any(|body| {
        let index = output_paragraphs.partition_point(|paragraph| paragraph.end <= body.start);
        !output_paragraphs
            .get(index)
            .is_some_and(|paragraph| paragraph.start <= body.start && body.end <= paragraph.end)
    }) || added_wrappers
        .into_iter()
        .any(|wrapper| !output_wrappers.contains(&emphasis_delimiter_key(&output, wrapper, marker)))
    {
        return Some(selection);
    }
    *text = output;
    result
}

fn emphasis_delimiter_key(
    text: &str,
    mut range: Range<usize>,
    marker: &str,
) -> (Range<usize>, usize) {
    // Nested *** syntax can assign the outer stars to either format. Accept
    // symmetric nesting, but not an unmatched literal prefix/suffix of stars.
    let center = range.start + range.end;
    let delimiter = marker.as_bytes()[0];
    while range.start > 0 && text.as_bytes()[range.start - 1] == delimiter {
        range.start -= 1;
    }
    while text.as_bytes().get(range.end) == Some(&delimiter) {
        range.end += 1;
    }
    (range, center)
}

fn toggle_wrap(
    text: &mut String,
    selection: Range<usize>,
    before: &str,
    after: &str,
) -> Range<usize> {
    let start_byte = char_to_byte(text, selection.start);
    let end_byte = char_to_byte(text, selection.end);
    let before_chars = before.chars().count();

    let has_wrapper = start_byte >= before.len()
        && text.get(start_byte - before.len()..start_byte) == Some(before)
        && text.get(end_byte..end_byte + after.len()) == Some(after);
    if has_wrapper {
        text.replace_range(end_byte..end_byte + after.len(), "");
        text.replace_range(start_byte - before.len()..start_byte, "");
        let start = selection.start.saturating_sub(before_chars);
        return start..selection.end.saturating_sub(before_chars);
    }

    text.insert_str(end_byte, after);
    text.insert_str(start_byte, before);
    if selection.is_empty() {
        let cursor = selection.start + before_chars;
        cursor..cursor
    } else {
        selection.start + before_chars..selection.end + before_chars
    }
}

fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for byte in text.bytes() {
        if byte == b'`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn inline_code_wrappers(content: &str) -> (String, String) {
    let fence = "`".repeat(longest_backtick_run(content) + 1);
    let needs_padding = content.starts_with('`')
        || content.ends_with('`')
        || content.starts_with(' ')
            && content.ends_with(' ')
            && content.bytes().any(|byte| byte != b' ');
    let padding = if needs_padding { " " } else { "" };
    (format!("{fence}{padding}"), format!("{padding}{fence}"))
}

fn toggle_inline_code(text: &mut String, selection: Range<usize>) -> Range<usize> {
    let mut selected = char_to_byte(text, selection.start)..char_to_byte(text, selection.end);
    let wrapper = pulldown_cmark::Parser::new_ext(text, crate::markdown::parser_options())
        .into_offset_iter()
        .find_map(|(event, syntax)| {
            if !matches!(event, pulldown_cmark::Event::Code(_)) {
                return None;
            }
            let marker_len = text[syntax.clone()]
                .bytes()
                .take_while(|byte| *byte == b'`')
                .count();
            let mut body = syntax.start + marker_len..syntax.end - marker_len;
            let raw = &text[body.clone()];
            // CommonMark strips one padding space only when both ends are
            // spaces and the body contains a non-space character.
            if raw.starts_with(' ') && raw.ends_with(' ') && raw.bytes().any(|byte| byte != b' ') {
                body.start += 1;
                body.end -= 1;
            }
            (body == selected).then_some(syntax)
        });
    if let Some(wrapper) = wrapper {
        let start = text[..wrapper.start].chars().count();
        let content = text[selected].to_owned();
        let end = start + content.chars().count();
        text.replace_range(wrapper, &content);
        return start..end;
    }
    if selection.is_empty() {
        return toggle_wrap(text, selection, "`", "`");
    }

    // A generated pair may not parse as inline code in its surrounding Markdown
    // (for example after a backslash or inside a fenced block).
    let (before, after) = inline_code_wrappers(&text[selected.clone()]);
    if selected.start >= before.len()
        && text.get(selected.start - before.len()..selected.start) == Some(before.as_str())
        && text.get(selected.end..selected.end + after.len()) == Some(after.as_str())
    {
        return toggle_wrap(text, selection, &before, &after);
    }

    // Adjacent source backticks would merge with the inserted fence, and a
    // later toggle would then remove them as syntax. Include those runs in
    // the content selection so they stay literal and can be restored intact.
    selected.start -= text[..selected.start]
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'`')
        .count();
    selected.end += text[selected.end..]
        .bytes()
        .take_while(|byte| *byte == b'`')
        .count();
    let selection = byte_range_to_char_range(text, selected.clone());
    let (before, after) = inline_code_wrappers(&text[selected]);
    toggle_wrap(text, selection, &before, &after)
}

fn toggle_code_block(text: &mut String, selection: Range<usize>) -> Range<usize> {
    let selected = char_to_byte(text, selection.start)..char_to_byte(text, selection.end);
    let wrapper = pulldown_cmark::Parser::new_ext(text, crate::markdown::parser_options())
        .into_offset_iter()
        .find_map(|(event, syntax)| {
            if !matches!(
                event,
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(
                    pulldown_cmark::CodeBlockKind::Fenced(_)
                ))
            ) {
                return None;
            }
            let fragment = &text[syntax.clone()];
            let opening_end = fragment.find('\n')?;
            let opening = fragment[..opening_end].trim_start_matches(' ');
            let marker = opening.chars().next()?;
            let marker_len = opening
                .chars()
                .take_while(|character| *character == marker)
                .count();
            let end = syntax.start + fragment.trim_end_matches(['\r', '\n']).len();
            let closing_start = syntax.start + text[syntax.start..end].rfind('\n')? + 1;
            let closing_line = &text[closing_start..end];
            let closing = closing_line.trim_start_matches(' ');
            if closing_line.len() - closing.len() > 3 {
                return None;
            }
            let closing_len = closing
                .chars()
                .take_while(|character| *character == marker)
                .count();
            if closing_len < marker_len || !closing[closing_len..].trim().is_empty() {
                return None;
            }
            let body_start = syntax.start + opening_end + 1;
            let body_end = if closing_start > body_start {
                line_break_before(text, closing_start)?.start
            } else {
                body_start
            };
            (selected == (body_start..body_end)).then_some(syntax.start..end)
        });
    if let Some(wrapper) = wrapper {
        let start = text[..wrapper.start].chars().count();
        let content = text[selected].to_owned();
        let end = start + content.chars().count();
        text.replace_range(wrapper, &content);
        return start..end;
    }

    let fence = "`".repeat((longest_backtick_run(&text[selected.clone()]) + 1).max(3));
    let leading = if selected.start > 0 && text.as_bytes()[selected.start - 1] != b'\n' {
        "\n\n"
    } else {
        ""
    };
    let trailing = if selected.end < text.len() && text.as_bytes()[selected.end] != b'\n' {
        "\n\n"
    } else {
        ""
    };
    toggle_wrap(
        text,
        selection,
        &format!("{leading}{fence}\n"),
        &format!("\n{fence}{trailing}"),
    )
}

fn insert_link(text: &mut String, selection: Range<usize>) -> Range<usize> {
    let start_byte = char_to_byte(text, selection.start);
    let end_byte = char_to_byte(text, selection.end);
    if selection.is_empty() {
        text.insert_str(start_byte, "[](https://)");
        let cursor = selection.start + 1;
        return cursor..cursor;
    }
    let label = escape_selected_link_label(&text[start_byte..end_byte]);
    let end = selection.start + 1 + label.chars().count();
    text.replace_range(start_byte..end_byte, &format!("[{label}](https://)"));
    selection.start + 1..end
}

fn escape_selected_link_label(label: &str) -> String {
    let literal_ranges = pulldown_cmark::Parser::new_ext(label, crate::markdown::parser_options())
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(
                event,
                pulldown_cmark::Event::Code(_)
                    | pulldown_cmark::Event::Html(_)
                    | pulldown_cmark::Event::InlineHtml(_)
            )
            .then_some(range)
        })
        .collect::<Vec<_>>();
    let mut literal = literal_ranges.iter().peekable();
    let mut output = String::with_capacity(label.len());
    let mut cursor = 0;
    while cursor < label.len() {
        if let Some(range) = literal.peek()
            && range.start == cursor
        {
            output.push_str(&label[(**range).clone()]);
            cursor = range.end;
            literal.next();
            continue;
        }
        let mut characters = label[cursor..].chars();
        let character = characters.next().expect("cursor precedes label end");
        cursor += character.len_utf8();
        if character == '\\' {
            output.push(character);
            if let Some(escaped) = characters.next() {
                output.push(escaped);
                cursor += escaped.len_utf8();
            } else {
                // A literal final backslash must not escape the new closing ].
                output.push('\\');
            }
        } else {
            if matches!(character, '[' | ']') {
                output.push('\\');
            }
            output.push(character);
        }
    }
    output
}

#[derive(Clone, Copy)]
enum LineCommand {
    Heading(u8),
    Quote,
    BulletList,
    OrderedList,
}

fn transform_selected_lines(
    text: &mut String,
    selection: Range<usize>,
    command: LineCommand,
) -> Range<usize> {
    let collapsed = selection.is_empty();
    let selection_start_byte = char_to_byte(text, selection.start);
    let selection_end_byte = char_to_byte(text, selection.end);
    let Range {
        start: block_start,
        end: block_end,
    } = selected_line_bytes(text, selection_start_byte..selection_end_byte);
    let original = text[block_start..block_end].to_owned();
    let lines = original.split('\n').collect::<Vec<_>>();
    let all_prefixed = lines
        .iter()
        .all(|line| line_has_command_prefix(line, command));
    let transformed = lines
        .iter()
        .enumerate()
        .map(|(index, line)| transform_line(line, command, all_prefixed, index + 1))
        .collect::<Vec<_>>()
        .join("\n");
    text.replace_range(block_start..block_end, &transformed);

    let start = text[..block_start].chars().count();
    if collapsed {
        let mapped = map_prefix_edit_cursor(
            &original,
            &transformed,
            selection_start_byte.saturating_sub(block_start),
        );
        start + mapped..start + mapped
    } else {
        start..start + transformed.chars().count()
    }
}

fn selected_line_bytes(text: &str, selection: Range<usize>) -> Range<usize> {
    let start = text[..selection.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let end = if !selection.is_empty() && text.as_bytes().get(selection.end - 1) == Some(&b'\n') {
        selection.end - 1
    } else {
        text[selection.end..]
            .find('\n')
            .map_or(text.len(), |index| selection.end + index)
    };
    start..end
}

fn map_prefix_edit_cursor(original: &str, transformed: &str, original_byte_offset: usize) -> usize {
    let original_offset = original[..original_byte_offset.min(original.len())]
        .chars()
        .count();
    let original_chars = original.chars().collect::<Vec<_>>();
    let transformed_chars = transformed.chars().collect::<Vec<_>>();
    let common_suffix = original_chars
        .iter()
        .rev()
        .zip(transformed_chars.iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let original_prefix = original_chars.len().saturating_sub(common_suffix);
    let transformed_prefix = transformed_chars.len().saturating_sub(common_suffix);
    if original_offset <= original_prefix {
        transformed_prefix
    } else {
        transformed_prefix + original_offset - original_prefix
    }
}

fn escape_link_label(label: &str) -> String {
    label
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace(['\r', '\n'], " ")
}

fn escape_link_destination(destination: &str) -> String {
    destination
        .replace(' ', "%20")
        .replace('<', "%3C")
        .replace('>', "%3E")
        .replace('(', "%28")
        .replace(')', "%29")
        .replace(['\r', '\n'], "")
}

fn line_has_command_prefix(line: &str, command: LineCommand) -> bool {
    match command {
        LineCommand::Heading(level) => {
            line.starts_with(&format!("{} ", "#".repeat(level as usize)))
        }
        LineCommand::Quote => line.starts_with("> "),
        LineCommand::BulletList => {
            line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ")
        }
        LineCommand::OrderedList => strip_ordered_prefix(line).is_some(),
    }
}

fn transform_line(line: &str, command: LineCommand, remove: bool, ordinal: usize) -> String {
    match command {
        LineCommand::Heading(level) => {
            let without_heading = strip_heading_prefix(line);
            if remove {
                without_heading.to_owned()
            } else {
                format!("{} {without_heading}", "#".repeat(level as usize))
            }
        }
        LineCommand::Quote => {
            if remove {
                line.strip_prefix("> ").unwrap_or(line).to_owned()
            } else {
                format!("> {line}")
            }
        }
        LineCommand::BulletList => {
            if remove {
                line.get(2..).unwrap_or(line).to_owned()
            } else {
                format!("- {line}")
            }
        }
        LineCommand::OrderedList => {
            if remove {
                strip_ordered_prefix(line).unwrap_or(line).to_owned()
            } else {
                format!("{ordinal}. {line}")
            }
        }
    }
}

fn strip_heading_prefix(line: &str) -> &str {
    let hash_count = line.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&hash_count) && line.as_bytes().get(hash_count) == Some(&b' ') {
        &line[hash_count + 1..]
    } else {
        line
    }
}

fn strip_ordered_prefix(line: &str) -> Option<&str> {
    let digit_count = line.bytes().take_while(u8::is_ascii_digit).count();
    let delimiter = line.as_bytes().get(digit_count);
    (digit_count > 0
        && matches!(delimiter, Some(b'.' | b')'))
        && line.as_bytes().get(digit_count + 1) == Some(&b' '))
    .then(|| &line[digit_count + 2..])
}

struct ContinuationPrefix {
    source_prefix_len: usize,
    content_start: usize,
    next_prefix: String,
}

fn continuation_prefix(line: &str) -> Option<ContinuationPrefix> {
    let indent_len = line
        .bytes()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    let mut cursor = indent_len;
    while line[cursor..].starts_with("> ") {
        cursor += 2;
    }
    let structural_prefix = &line[..cursor];
    let rest = &line[cursor..];

    for marker in ["- ", "* ", "+ "] {
        if let Some(after_marker) = rest.strip_prefix(marker) {
            let task_len = if after_marker.starts_with("[ ] ")
                || after_marker.starts_with("[x] ")
                || after_marker.starts_with("[X] ")
            {
                4
            } else {
                0
            };
            let source_prefix_len = cursor + marker.len() + task_len;
            let next_marker = if task_len > 0 {
                format!("{marker}[ ] ")
            } else {
                marker.to_owned()
            };
            return Some(ContinuationPrefix {
                source_prefix_len,
                content_start: source_prefix_len,
                next_prefix: format!("{structural_prefix}{next_marker}"),
            });
        }
    }

    let digit_count = rest.bytes().take_while(u8::is_ascii_digit).count();
    let delimiter = rest.as_bytes().get(digit_count).copied();
    if digit_count > 0
        && matches!(delimiter, Some(b'.' | b')'))
        && rest.as_bytes().get(digit_count + 1) == Some(&b' ')
    {
        let ordinal = rest[..digit_count].parse::<u64>().unwrap_or(0);
        let source_prefix_len = cursor + digit_count + 2;
        return Some(ContinuationPrefix {
            source_prefix_len,
            content_start: source_prefix_len,
            next_prefix: format!(
                "{structural_prefix}{}{} ",
                ordinal.saturating_add(1),
                char::from(delimiter.unwrap_or(b'.'))
            ),
        });
    }

    (cursor > indent_len).then(|| ContinuationPrefix {
        source_prefix_len: cursor,
        content_start: cursor,
        next_prefix: structural_prefix.to_owned(),
    })
}

fn is_probable_url(text: &str) -> bool {
    let text = text.trim();
    !text.contains(char::is_whitespace)
        && ["https://", "http://", "mailto:", "file://"]
            .iter()
            .any(|prefix| text.starts_with(prefix))
}

fn clamp_char_range(text: &str, range: Range<usize>) -> Range<usize> {
    let length = text.chars().count();
    range.start.min(length).min(range.end)..range.start.max(range.end).min(length)
}

pub(crate) fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte_index, _)| byte_index)
}

/// Finds the complete line break ending at a UTF-8 byte boundary.
pub(crate) fn line_break_before(source: &str, byte_index: usize) -> Option<Range<usize>> {
    let before = source.get(..byte_index)?;
    if before.ends_with("\r\n") {
        Some(byte_index - 2..byte_index)
    } else if before.ends_with(['\n', '\r']) {
        Some(byte_index - 1..byte_index)
    } else {
        None
    }
}

fn byte_range_to_char_range(text: &str, range: Range<usize>) -> Range<usize> {
    text[..range.start].chars().count()..text[..range.end].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_break_boundaries_preserve_unicode_and_all_supported_newlines() {
        for newline in ["\n", "\r", "\r\n"] {
            let source = format!("中文🙂{newline}后文");
            let start = "中文🙂".len();
            let end = start + newline.len();
            assert_eq!(line_break_before(&source, end), Some(start..end));
            for invalid in [0, 1, start, end + 1, source.len(), source.len() + 1] {
                assert_eq!(line_break_before(&source, invalid), None);
            }
        }
    }

    #[test]
    fn inline_code_command_preserves_backticks_spaces_and_can_be_toggled_off() {
        for original in ["a`b", "`", "``", " 中文`🙂 ", "   "] {
            let mut source = original.to_owned();
            let selected = apply_markdown_command(
                &mut source,
                0..original.chars().count(),
                MarkdownCommand::InlineCode,
            );
            let codes = pulldown_cmark::Parser::new_ext(&source, crate::markdown::parser_options())
                .filter_map(|event| match event {
                    pulldown_cmark::Event::Code(code) => Some(code.into_string()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(codes, [original], "generated Markdown: {source:?}");
            assert_eq!(
                &source[char_to_byte(&source, selected.start)..char_to_byte(&source, selected.end)],
                original,
                "the selection must address the original content, not synthetic padding",
            );
            let unwrapped =
                apply_markdown_command(&mut source, selected, MarkdownCommand::InlineCode);
            assert_eq!(source, original);
            assert_eq!(unwrapped, 0..original.chars().count());
        }
    }

    #[test]
    fn inline_code_command_preserves_unicode_selection_and_empty_cursor() {
        let original = "前甲`🙂后";
        let mut source = original.to_owned();
        let selected = apply_markdown_command(&mut source, 1..4, MarkdownCommand::InlineCode);
        assert_eq!(
            &source[char_to_byte(&source, selected.start)..char_to_byte(&source, selected.end)],
            "甲`🙂",
        );
        assert!(source.starts_with('前') && source.ends_with('后'));
        assert_eq!(
            crate::wysiwyg::VisualProjection::from_markdown(&source).text(),
            original,
        );
        assert_eq!(
            apply_markdown_command(&mut source, selected, MarkdownCommand::InlineCode),
            1..4,
        );
        assert_eq!(source, original);

        let mut source = "前后".to_owned();
        let cursor = apply_markdown_command(&mut source, 1..1, MarkdownCommand::InlineCode);
        assert_eq!(source, "前``后");
        assert_eq!(cursor, 2..2);
        assert_eq!(
            apply_markdown_command(&mut source, cursor, MarkdownCommand::InlineCode),
            1..1,
        );
        assert_eq!(source, "前后");
    }

    #[test]
    fn inline_code_command_keeps_adjacent_backticks_as_content() {
        for (original, selection, expanded) in [
            ("```", 1..2, 0..3),
            ("前`中`🙂`后", 2..5, 1..6),
            ("前``🙂后", 3..4, 1..4),
            ("前🙂``后", 1..2, 1..4),
        ] {
            let mut source = original.to_owned();
            let expected_content = &original
                [char_to_byte(original, expanded.start)..char_to_byte(original, expanded.end)];
            let selected =
                apply_markdown_command(&mut source, selection, MarkdownCommand::InlineCode);
            assert_eq!(
                &source[char_to_byte(&source, selected.start)..char_to_byte(&source, selected.end)],
                expected_content,
                "neighboring backticks must remain content rather than join the new fence",
            );
            let codes = pulldown_cmark::Parser::new_ext(&source, crate::markdown::parser_options())
                .filter_map(|event| match event {
                    pulldown_cmark::Event::Code(code) => Some(code.into_string()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(codes, [expected_content]);
            assert_eq!(
                apply_markdown_command(&mut source, selected, MarkdownCommand::InlineCode),
                expanded,
            );
            assert_eq!(source, original);
        }
    }

    #[test]
    fn inline_code_command_removes_nonminimal_fences() {
        for original in ["``foo``", "`` foo ``", "```foo```"] {
            let mut source = original.to_owned();
            let start = original.find("foo").unwrap();
            let selected =
                apply_markdown_command(&mut source, start..start + 3, MarkdownCommand::InlineCode);
            assert_eq!(source, "foo");
            assert_eq!(selected, 0..3);
            assert_eq!(
                apply_markdown_command(&mut source, selected, MarkdownCommand::InlineCode),
                1..4,
            );
            assert_eq!(source, "`foo`");
        }
    }

    #[test]
    fn inline_code_command_round_trips_when_context_hides_the_fence() {
        for (original, selection) in [("\\中🙂", 1..3), ("```\n中🙂\n```", 4..6)] {
            let mut source = original.to_owned();
            let selected =
                apply_markdown_command(&mut source, selection.clone(), MarkdownCommand::InlineCode);
            assert_eq!(
                &source[char_to_byte(&source, selected.start)..char_to_byte(&source, selected.end)],
                "中🙂",
            );
            assert_eq!(
                apply_markdown_command(&mut source, selected, MarkdownCommand::InlineCode),
                selection,
            );
            assert_eq!(source, original);
        }
    }

    #[test]
    fn link_command_escapes_selected_labels_and_selects_the_escaped_unicode_source() {
        for (original, escaped) in [
            ("a]b", r"a\]b"),
            ("[中文]🙂", r"\[中文\]🙂"),
            (r"C:\文档", r"C:\文档"),
        ] {
            let mut source = original.to_owned();
            let selected = apply_markdown_command(
                &mut source,
                0..original.chars().count(),
                MarkdownCommand::Link,
            );
            let links = pulldown_cmark::Parser::new_ext(&source, crate::markdown::parser_options())
                .filter_map(|event| match event {
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
                        dest_url, ..
                    }) => Some(dest_url.into_string()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(links, ["https://"], "generated Markdown: {source:?}");
            assert_eq!(
                crate::wysiwyg::VisualProjection::from_markdown(&source).text(),
                original,
            );
            assert_eq!(
                &source[char_to_byte(&source, selected.start)..char_to_byte(&source, selected.end)],
                escaped,
            );
        }
    }

    #[test]
    fn link_command_keeps_existing_markdown_escapes_and_literal_code() {
        for original in [r"\*literal\*", r"\\*emphasis*", "`a[b]`", r"tail\"] {
            let expected = crate::wysiwyg::VisualProjection::from_markdown(original)
                .text()
                .to_owned();
            let mut source = original.to_owned();
            apply_markdown_command(
                &mut source,
                0..original.chars().count(),
                MarkdownCommand::Link,
            );
            assert_eq!(
                crate::wysiwyg::VisualProjection::from_markdown(&source).text(),
                expected,
                "generated Markdown: {source:?}",
            );
            assert!(
                pulldown_cmark::Parser::new_ext(&source, crate::markdown::parser_options()).any(
                    |event| matches!(
                        event,
                        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { .. })
                    )
                ),
                "the selected Markdown must remain a valid link label: {source:?}",
            );
        }
    }

    #[test]
    fn code_block_removal_preserves_unicode_body_for_each_line_ending() {
        for newline in ["\n", "\r\n"] {
            for fence in ["```rust", "~~~text"] {
                for body in [
                    String::new(),
                    "中文🙂".to_owned(),
                    format!("中文🙂{newline}第二行"),
                ] {
                    let closer = &fence[..3];
                    let mut source = format!("{fence}{newline}{body}{newline}{closer}");
                    let start = fence.chars().count() + newline.len();
                    let selected = apply_markdown_command(
                        &mut source,
                        start..start + body.chars().count(),
                        MarkdownCommand::CodeBlock,
                    );
                    assert_eq!(source, body, "newline={newline:?} fence={fence}");
                    assert_eq!(selected, 0..body.chars().count());
                }
            }
        }
    }

    #[test]
    fn code_block_command_preserves_embedded_fences_and_can_be_toggled_off() {
        for original in ["first\n```\nlast", "中文🙂\n````\nend"] {
            let mut source = original.to_owned();
            let selected = apply_markdown_command(
                &mut source,
                0..original.chars().count(),
                MarkdownCommand::CodeBlock,
            );
            assert_eq!(crate::wysiwyg::fenced_code_content(&source), Some(original));
            assert_eq!(
                &source[char_to_byte(&source, selected.start)..char_to_byte(&source, selected.end)],
                original,
            );
            let unwrapped =
                apply_markdown_command(&mut source, selected, MarkdownCommand::CodeBlock);
            assert_eq!(source, original);
            assert_eq!(unwrapped, 0..original.chars().count());
        }
    }

    #[test]
    fn code_block_command_at_a_midline_caret_preserves_the_surrounding_prose() {
        let mut source = "before after".to_owned();
        let cursor = apply_markdown_command(&mut source, 7..7, MarkdownCommand::CodeBlock);
        let projected = crate::wysiwyg::VisualProjection::from_markdown(&source);
        assert!(projected.text().contains("before"), "source: {source:?}");
        assert!(projected.text().contains("after"), "source: {source:?}");
        let cursor_byte = char_to_byte(&source, cursor.end);
        assert!(
            pulldown_cmark::Parser::new_ext(&source, crate::markdown::parser_options())
                .into_offset_iter()
                .any(|(event, range)| matches!(
                    event,
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(_))
                ) && range.contains(&cursor_byte)),
            "caret must be inside the inserted code block: {source:?} at {cursor:?}",
        );
    }

    #[test]
    fn code_block_command_preserves_a_unicode_line_selection_when_toggled() {
        let original = "前\n甲🙂\n后";
        let mut source = original.to_owned();
        let selected = apply_markdown_command(&mut source, 2..4, MarkdownCommand::CodeBlock);
        assert_eq!(
            &source[char_to_byte(&source, selected.start)..char_to_byte(&source, selected.end)],
            "甲🙂",
        );
        assert_eq!(
            apply_markdown_command(&mut source, selected, MarkdownCommand::CodeBlock),
            2..4,
        );
        assert_eq!(source, original);
    }

    #[test]
    fn wraps_and_unwraps_unicode_selection() {
        let mut text = "你好 world".to_owned();
        let selection = apply_markdown_command(&mut text, 0..2, MarkdownCommand::Bold);
        assert_eq!(text, "**你好** world");
        assert_eq!(selection, 2..4);

        let selection = apply_markdown_command(&mut text, selection, MarkdownCommand::Bold);
        assert_eq!(text, "你好 world");
        assert_eq!(selection, 0..2);
    }

    #[test]
    fn single_line_strikethrough_rejects_an_opening_tilde_collision() {
        for original in ["~a", "~中🙂", "   ~中🙂", "- ~中🙂", "> ~中🙂", "# ~中🙂"]
        {
            let tilde = original
                .chars()
                .position(|character| character == '~')
                .unwrap();
            for start in [tilde, tilde + 1] {
                let selection = start..original.chars().count();
                let mut source = original.to_owned();
                let next = apply_markdown_command(
                    &mut source,
                    selection.clone(),
                    MarkdownCommand::Strikethrough,
                );
                assert_eq!(
                    source, original,
                    "a delimiter collision must not create code or literal markers"
                );
                assert_eq!(next, selection);
            }
        }
    }

    #[test]
    fn single_line_strikethrough_keeps_heading_list_and_quote_toggles() {
        for (original, selection, expected) in [
            ("中文🙂", 0..3, "~~中文🙂~~"),
            ("# 中文🙂", 2..5, "# ~~中文🙂~~"),
            ("- 中文🙂", 2..5, "- ~~中文🙂~~"),
            ("> 中文🙂", 2..5, "> ~~中文🙂~~"),
            ("前~中🙂后", 0..5, "~~前~中🙂后~~"),
        ] {
            let mut source = original.to_owned();
            let next = apply_markdown_command(
                &mut source,
                selection.clone(),
                MarkdownCommand::Strikethrough,
            );
            assert_eq!(source, expected);
            let html = crate::markdown::render_html_fragment(&source);
            assert_eq!(html.matches("<del>").count(), 1, "{html}");
            assert_eq!(
                apply_markdown_command(&mut source, next, MarkdownCommand::Strikethrough),
                selection
            );
            assert_eq!(source, original);
        }
    }

    #[test]
    fn multiline_strikethrough_never_turns_a_literal_tilde_into_a_code_fence() {
        for newline in ["\n", "\r", "\r\n"] {
            for (initial, start) in [
                (format!("~a{newline}b"), 0),
                (
                    format!("前{newline}~中🙂{newline}后"),
                    1 + newline.chars().count(),
                ),
                (format!("前~a{newline}b"), 1),
            ] {
                let mut source = initial.clone();
                let selected = start..initial.chars().count();
                let next =
                    apply_markdown_command(&mut source, selected, MarkdownCommand::Strikethrough);
                assert_eq!(
                    source, initial,
                    "an unsafe delimiter must leave the paragraph intact"
                );
                assert!(
                    !pulldown_cmark::Parser::new_ext(&source, crate::markdown::parser_options())
                        .any(|event| matches!(
                            event,
                            pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(_))
                        )),
                    "strikethrough changed a paragraph into code: {source:?}",
                );
                apply_markdown_command(&mut source, next, MarkdownCommand::Strikethrough);
                assert_eq!(source, initial);
            }
        }
    }

    #[test]
    fn emphasis_formats_soft_line_breaks_and_interior_literal_tildes() {
        for (command, marker, tag) in [
            (MarkdownCommand::Bold, "**", "strong"),
            (MarkdownCommand::Italic, "*", "em"),
            (MarkdownCommand::Strikethrough, "~~", "del"),
        ] {
            for newline in ["\n", "\r", "\r\n"] {
                for body in [format!("中🙂{newline}文"), format!("前~a{newline}b~后")] {
                    let mut source = body.clone();
                    let selected =
                        apply_markdown_command(&mut source, 0..body.chars().count(), command);
                    assert_eq!(source, format!("{marker}{body}{marker}"));
                    let html = crate::markdown::render_html_fragment(&source);
                    assert_eq!(html.matches(&format!("<{tag}>")).count(), 1, "{html}");
                    assert_eq!(
                        apply_markdown_command(&mut source, selected, command),
                        0..body.chars().count(),
                    );
                    assert_eq!(source, body);
                }
            }
        }
    }

    #[test]
    fn emphasis_commands_format_each_selected_paragraph_and_toggle_back() {
        for (command, marker, tag) in [
            (MarkdownCommand::Bold, "**", "strong"),
            (MarkdownCommand::Italic, "*", "em"),
            (MarkdownCommand::Strikethrough, "~~", "del"),
        ] {
            for (source, selection, expected) in [
                (
                    "中🙂\n\n文",
                    0..5,
                    format!("{marker}中🙂{marker}\n\n{marker}文{marker}"),
                ),
                (
                    "甲乙\n\n丙丁",
                    1..5,
                    format!("甲{marker}乙{marker}\n\n{marker}丙{marker}丁"),
                ),
                (
                    "> 中文\n>\n> 后文",
                    0..12,
                    format!("> {marker}中文{marker}\n>\n> {marker}后文{marker}"),
                ),
                (
                    "- 中文\n\n- 后文",
                    0..10,
                    format!("- {marker}中文{marker}\n\n- {marker}后文{marker}"),
                ),
                (
                    "`中文`\n\n后文",
                    0..8,
                    format!("{marker}`中文`{marker}\n\n{marker}后文{marker}"),
                ),
            ] {
                let mut text = source.to_owned();
                let next = apply_markdown_command(&mut text, selection, command);
                assert_eq!(text, expected, "{command:?} source={source:?}");
                let html = crate::markdown::render_html_fragment(&text);
                assert_eq!(html.matches(&format!("<{tag}>")).count(), 2, "{html}");
                apply_markdown_command(&mut text, next, command);
                assert_eq!(text, source, "toggle {command:?}");
            }
        }
    }

    #[test]
    fn italic_on_bold_text_keeps_both_formats_and_toggles_back() {
        let mut text = "**中文🙂**".to_owned();
        let next = apply_markdown_command(&mut text, 2..5, MarkdownCommand::Italic);
        let html = crate::markdown::render_html_fragment(&text);
        assert!(html.contains("<strong>") && html.contains("<em>"), "{html}");
        apply_markdown_command(&mut text, next, MarkdownCommand::Italic);
        assert_eq!(text, "**中文🙂**");
    }

    #[test]
    fn strikethrough_around_multiline_emphasis_toggles_back() {
        let original = "a*=\na*";
        let mut source = original.to_owned();
        let selected = apply_markdown_command(&mut source, 1..6, MarkdownCommand::Strikethrough);
        assert_eq!(source, "a~~*=\na*~~");
        assert_eq!(selected, 3..8);
        apply_markdown_command(&mut source, selected, MarkdownCommand::Strikethrough);
        assert_eq!(source, original);
    }

    #[test]
    fn italic_partial_multiline_wrapper_preserves_nested_markers() {
        let original = "*a\n*a**";
        let mut source = original.to_owned();
        let selected = apply_markdown_command(&mut source, 0..5, MarkdownCommand::Italic);
        assert_eq!(source, original);
        apply_markdown_command(&mut source, selected, MarkdownCommand::Italic);
        assert_eq!(source, original);
    }

    #[test]
    fn multi_paragraph_emphasis_preserves_nested_formatting_on_toggle() {
        for source in [
            "a\n\n*b*",
            "*a*\n\nb",
            "a\n\n*b*\n\nc",
            "a\n\n**b**",
            "**a**\n\nb",
            "a\n\n~~b~~",
            "`a`\n\nb",
            "a\n\n`b`",
            "`a\nb`\r\rc",
            "a\r\rb",
            "a\r\n\r\nb",
            "<i>a</i>\n\nb",
            "$a$\n\nb",
            "a\\\n\nb",
            "[a](b)\n\nc",
            "a\n\n**",
            "**# a**\n\nb",
            "- a\n\n- b",
            "a\n\n**b",
        ] {
            for command in [
                MarkdownCommand::Bold,
                MarkdownCommand::Italic,
                MarkdownCommand::Strikethrough,
            ] {
                for start in 0..=source.chars().count() {
                    for end in start..=source.chars().count() {
                        let mut text = source.to_owned();
                        let next = apply_markdown_command(&mut text, start..end, command);
                        apply_markdown_command(&mut text, next, command);
                        assert_eq!(
                            text, source,
                            "{source:?} {command:?} selection={start}..{end}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn emphasis_commands_keep_selection_whitespace_outside_rendered_markers() {
        for (command, marker, tag) in [
            (MarkdownCommand::Bold, "**", "strong"),
            (MarkdownCommand::Italic, "*", "em"),
            (MarkdownCommand::Strikethrough, "~~", "del"),
        ] {
            for (original, selection, expected, body_start, body) in [
                (
                    "hello world",
                    5..11,
                    format!("hello {marker}world{marker}"),
                    6,
                    "world",
                ),
                (
                    "hello world ",
                    6..12,
                    format!("hello {marker}world{marker} "),
                    6,
                    "world",
                ),
                (
                    "前　中🙂 后",
                    1..5,
                    format!("前　{marker}中🙂{marker} 后"),
                    2,
                    "中🙂",
                ),
            ] {
                let mut source = original.to_owned();
                let selected = apply_markdown_command(&mut source, selection, command);
                let html = crate::markdown::render_html_fragment(&source);
                assert!(
                    html.contains(&format!("<{tag}>{body}</{tag}>")),
                    "{command:?} must render the selected text: {source:?} => {html:?}",
                );
                assert_eq!(source, expected);
                assert_eq!(
                    selected,
                    body_start + marker.len()..body_start + marker.len() + body.chars().count(),
                );
                let restored = apply_markdown_command(&mut source, selected, command);
                assert_eq!(source, original);
                assert_eq!(restored, body_start..body_start + body.chars().count());
            }
        }
    }

    #[test]
    fn emphasis_commands_leave_whitespace_only_selections_unchanged() {
        for command in [
            MarkdownCommand::Bold,
            MarkdownCommand::Italic,
            MarkdownCommand::Strikethrough,
        ] {
            let mut source = "前　 \t后".to_owned();
            assert_eq!(apply_markdown_command(&mut source, 1..4, command), 1..4);
            assert_eq!(source, "前　 \t后");

            let mut source = "前后".to_owned();
            let cursor = apply_markdown_command(&mut source, 1..1, command);
            assert!(cursor.is_empty());
            assert_eq!(apply_markdown_command(&mut source, cursor, command), 1..1);
            assert_eq!(source, "前后");
        }
    }

    #[test]
    fn toggles_heading_without_stacking_prefixes() {
        let mut text = "## title\nbody".to_owned();
        let selection = apply_markdown_command(&mut text, 0..0, MarkdownCommand::Heading(3));
        assert_eq!(text, "### title\nbody");
        assert_eq!(selection, 1..1);
        let selection = apply_markdown_command(&mut text, selection, MarkdownCommand::Heading(3));
        assert_eq!(text, "title\nbody");
        assert_eq!(selection, 0..0);
    }

    #[test]
    fn creates_and_removes_ordered_lists() {
        let mut text = "one\ntwo".to_owned();
        let selected = apply_markdown_command(&mut text, 0..7, MarkdownCommand::OrderedList);
        assert_eq!(text, "1. one\n2. two");
        apply_markdown_command(&mut text, selected, MarkdownCommand::OrderedList);
        assert_eq!(text, "one\ntwo");
    }

    #[test]
    fn search_wraps_and_handles_chinese() {
        let text = "Alpha 中文 alpha";
        assert_eq!(find_next(text, "中文", 0, false), Some(6..8));
        assert_eq!(find_next(text, "alpha", 6, false), Some(9..14));
        assert_eq!(find_next(text, "Alpha", 14, true), Some(0..5));
    }

    #[test]
    fn replaces_all_without_invalidating_later_ranges() {
        let mut text = "one ONE 中文 one".to_owned();
        let count = replace_all(&mut text, "one", "two", false);
        assert_eq!(count, 3);
        assert_eq!(text, "two two 中文 two");
    }

    #[test]
    fn maps_one_based_lines_to_unicode_character_offsets() {
        let text = "一行\nsecond\n第三行";
        assert_eq!(char_index_for_line(text, 1), 0);
        assert_eq!(char_index_for_line(text, 2), 3);
        assert_eq!(char_index_for_line(text, 3), 10);
        assert_eq!(char_index_for_line(text, 99), text.chars().count());
    }

    #[test]
    fn continues_lists_quotes_ordering_and_tasks() {
        for (before, cursor, expected, next) in [
            ("- item\n", 7, "- item\n- ", 9),
            ("3. item\n", 8, "3. item\n4. ", 11),
            ("> quote\n", 8, "> quote\n> ", 10),
            ("- [x] done\n", 11, "- [x] done\n- [ ] ", 17),
            ("- [X] done\n", 11, "- [X] done\n- [ ] ", 17),
            ("  - 中文\n", 7, "  - 中文\n  - ", 11),
        ] {
            let mut text = before.to_owned();
            assert_eq!(continue_markdown_line(&mut text, cursor), Some(next..next));
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn replaces_code_editor_auto_indent_when_continuing_markdown() {
        for (before, expected) in [
            ("  - item\n  ", "  - item\n  - "),
            ("  9) item\n  ", "  9) item\n  10) "),
            ("  > quote\n  ", "  > quote\n  > "),
        ] {
            let mut text = before.to_owned();
            let cursor = text.chars().count();
            let selection = continue_markdown_line(&mut text, cursor).unwrap();
            assert_eq!(text, expected, "source: {before:?}");
            assert_eq!(selection.end, expected.chars().count());
        }
    }

    #[test]
    fn exits_an_empty_list_item() {
        let mut text = "- item\n- \n".to_owned();
        assert_eq!(continue_markdown_line(&mut text, 10), Some(8..8));
        assert_eq!(text, "- item\n\n");

        let mut nested = "  - item\n  - \n  ".to_owned();
        let cursor = nested.chars().count();
        assert_eq!(continue_markdown_line(&mut nested, cursor), Some(10..10));
        assert_eq!(nested, "  - item\n\n");
    }

    #[test]
    fn indents_and_outdents_selected_unicode_lines() {
        let mut text = "一\n二".to_owned();
        let selected = indent_selected_lines(&mut text, 0..3, false);
        assert_eq!(text, "    一\n    二");
        indent_selected_lines(&mut text, selected, true);
        assert_eq!(text, "一\n二");
    }

    #[test]
    fn indentation_and_line_formats_preserve_a_collapsed_cursor() {
        let mut text = "alpha".to_owned();
        let selection = indent_selected_lines(&mut text, 5..5, false);
        assert_eq!(text, "    alpha");
        assert_eq!(selection, 9..9);

        let selection = apply_markdown_command(&mut text, selection, MarkdownCommand::BulletList);
        assert_eq!(text, "-     alpha");
        assert_eq!(selection, 11..11);

        let mut text = "    alpha".to_owned();
        let selection = indent_selected_lines(&mut text, 9..9, true);
        assert_eq!(text, "alpha");
        assert_eq!(selection, 5..5);
    }

    #[test]
    fn toggles_parenthesized_ordered_lists_without_stacking_markers() {
        let mut text = "1) one\n2) two".to_owned();
        let end = text.chars().count();
        let selection = apply_markdown_command(&mut text, 0..end, MarkdownCommand::OrderedList);
        assert_eq!(text, "one\ntwo");
        apply_markdown_command(&mut text, selection, MarkdownCommand::OrderedList);
        assert_eq!(text, "1. one\n2. two");
    }

    #[test]
    fn smart_paste_wraps_selected_text_as_a_link() {
        let mut text = "Open documentation now".to_owned();
        let cursor = paste_url_as_markdown_link(&mut text, 5..18, "https://example.com").unwrap();
        assert_eq!(text, "Open [documentation](https://example.com) now");
        assert_eq!(cursor, 41..41);
        assert!(paste_url_as_markdown_link(&mut text, 0..0, "not a url").is_none());
    }

    #[test]
    fn smart_paste_escapes_labels_and_ambiguous_destinations() {
        let mut text = "a]b".to_owned();
        let cursor =
            paste_url_as_markdown_link(&mut text, 0..3, "https://example.com/a_(b)").unwrap();
        assert_eq!(text, "[a\\]b](https://example.com/a_%28b%29)");
        assert_eq!(cursor, text.chars().count()..text.chars().count());

        let mut encoded = "label".to_owned();
        paste_url_as_markdown_link(&mut encoded, 0..5, "https://example.com/a%20b").unwrap();
        assert_eq!(encoded, "[label](https://example.com/a%20b)");

        let mut resource = String::new();
        insert_resource_link(&mut resource, 0..0, "a]b", "assets/a (1).png", true);
        assert_eq!(resource, "![a\\]b](assets/a%20%281%29.png)");
    }

    #[test]
    fn smart_pairs_wrap_selections_and_skip_existing_closers() {
        let mut text = "中文".to_owned();
        assert_eq!(apply_smart_pair(&mut text, 0..2, "("), Some(1..3));
        assert_eq!(text, "(中文)");

        let mut text = "call()".to_owned();
        assert_eq!(apply_smart_pair(&mut text, 5..5, ")"), Some(6..6));
        assert_eq!(text, "call()");

        let mut text = String::new();
        assert_eq!(apply_smart_pair(&mut text, 0..0, "`"), None);
        assert!(text.is_empty());
    }
}
