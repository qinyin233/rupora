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
        MarkdownCommand::Bold => toggle_wrap(text, selection, "**", "**"),
        MarkdownCommand::Italic => toggle_wrap(text, selection, "*", "*"),
        MarkdownCommand::Strikethrough => toggle_wrap(text, selection, "~~", "~~"),
        MarkdownCommand::InlineCode => toggle_wrap(text, selection, "`", "`"),
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
        MarkdownCommand::CodeBlock => toggle_wrap(text, selection, "```\n", "\n```"),
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
    let mut matches = collect_matches(text, query, match_case);
    matches
        .iter()
        .rev()
        .find(|range| char_to_byte(text, range.end) <= before_byte)
        .cloned()
        .or_else(|| matches.pop())
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
    let matches = collect_matches(text, query, match_case);
    for range in matches.iter().rev() {
        let start = char_to_byte(text, range.start);
        let end = char_to_byte(text, range.end);
        text.replace_range(start..end, replacement);
    }
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

    candidate_byte_starts(text)
        .filter(|byte_start| *byte_start >= start_byte)
        .find_map(|byte_start| {
            let end = byte_start.checked_add(query.len())?;
            let candidate = text.get(byte_start..end)?;
            candidate
                .eq_ignore_ascii_case(query)
                .then(|| byte_range_to_char_range(text, byte_start..end))
        })
}

fn collect_matches(text: &str, query: &str, match_case: bool) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    if match_case {
        return text
            .match_indices(query)
            .map(|(start, matched)| byte_range_to_char_range(text, start..start + matched.len()))
            .collect();
    }

    let mut last_end = 0;
    candidate_byte_starts(text)
        .filter_map(|start| {
            if start < last_end {
                return None;
            }
            let end = start.checked_add(query.len())?;
            let candidate = text.get(start..end)?;
            if candidate.eq_ignore_ascii_case(query) {
                last_end = end;
                Some(byte_range_to_char_range(text, start..end))
            } else {
                None
            }
        })
        .collect()
}

fn candidate_byte_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    text.char_indices().map(|(index, _)| index)
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

fn insert_link(text: &mut String, selection: Range<usize>) -> Range<usize> {
    let start_byte = char_to_byte(text, selection.start);
    let end_byte = char_to_byte(text, selection.end);
    if selection.is_empty() {
        text.insert_str(start_byte, "[](https://)");
        let cursor = selection.start + 1;
        return cursor..cursor;
    }
    text.insert_str(end_byte, "](https://)");
    text.insert(start_byte, '[');
    selection.start + 1..selection.end + 1
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
    let selection_start_byte = char_to_byte(text, selection.start);
    let selection_end_byte = char_to_byte(text, selection.end);
    let block_start = text[..selection_start_byte]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let block_end = text[selection_end_byte..]
        .find('\n')
        .map_or(text.len(), |index| selection_end_byte + index);
    let original = &text[block_start..block_end];
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
    start..start + transformed.chars().count()
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
    (digit_count > 0 && line.get(digit_count..digit_count + 2) == Some(". "))
        .then(|| &line[digit_count + 2..])
}

fn clamp_char_range(text: &str, range: Range<usize>) -> Range<usize> {
    let length = text.chars().count();
    range.start.min(length).min(range.end)..range.start.max(range.end).min(length)
}

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte_index, _)| byte_index)
}

fn byte_range_to_char_range(text: &str, range: Range<usize>) -> Range<usize> {
    text[..range.start].chars().count()..text[..range.end].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn toggles_heading_without_stacking_prefixes() {
        let mut text = "## title\nbody".to_owned();
        apply_markdown_command(&mut text, 0..0, MarkdownCommand::Heading(3));
        assert_eq!(text, "### title\nbody");
        apply_markdown_command(&mut text, 0..0, MarkdownCommand::Heading(3));
        assert_eq!(text, "title\nbody");
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
}
