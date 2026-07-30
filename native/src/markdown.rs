use std::{
    collections::{HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownAnalysis {
    pub headings: Vec<Heading>,
    pub characters: usize,
    pub words: usize,
    pub lines: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub line: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockId(u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownBlock {
    pub id: BlockId,
    pub range: Range<usize>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct BlockIndex {
    source: String,
    blocks: Vec<MarkdownBlock>,
    next_id: u64,
}

impl BlockIndex {
    pub fn new(source: &str) -> Self {
        let mut next_id = 1;
        let blocks = block_ranges(source)
            .into_iter()
            .map(|range| {
                let id = BlockId(next_id);
                next_id += 1;
                block_from_range(source, id, range)
            })
            .collect();
        Self {
            source: source.to_owned(),
            blocks,
            next_id,
        }
    }

    pub fn blocks(&self) -> &[MarkdownBlock] {
        &self.blocks
    }

    pub fn update(&mut self, source: &str) {
        if self.source == source {
            return;
        }

        let new_ranges = block_ranges(source);
        let mut assigned = vec![None; new_ranges.len()];
        let mut exact_positions = HashMap::<u64, VecDeque<usize>>::new();
        for (index, block) in self.blocks.iter().enumerate() {
            exact_positions
                .entry(block_hash(&self.source[block.range.clone()]))
                .or_default()
                .push_back(index);
        }

        let mut old_used = vec![false; self.blocks.len()];
        let mut last_old = 0usize;
        for (new_index, range) in new_ranges.iter().enumerate() {
            let text = &source[range.clone()];
            let hash = block_hash(text);
            let Some(candidates) = exact_positions.get_mut(&hash) else {
                continue;
            };
            while candidates.front().is_some_and(|index| *index < last_old) {
                candidates.pop_front();
            }
            let Some(old_index) = candidates.iter().copied().find(|old_index| {
                !old_used[*old_index] && self.source[self.blocks[*old_index].range.clone()] == *text
            }) else {
                continue;
            };
            while candidates.front().is_some_and(|index| *index <= old_index) {
                candidates.pop_front();
            }
            assigned[new_index] = Some(self.blocks[old_index].id);
            old_used[old_index] = true;
            last_old = old_index + 1;
        }

        reconcile_changed_gaps(
            &self.source,
            &self.blocks,
            source,
            &new_ranges,
            &mut old_used,
            &mut assigned,
        );

        let mut next_id = self.next_id;
        self.blocks = new_ranges
            .into_iter()
            .enumerate()
            .map(|(index, range)| {
                let id = assigned[index].unwrap_or_else(|| {
                    let id = BlockId(next_id);
                    next_id += 1;
                    id
                });
                block_from_range(source, id, range)
            })
            .collect();
        self.next_id = next_id;
        self.source.clear();
        self.source.push_str(source);
    }
}

pub fn parser_options() -> Options {
    Options::ENABLE_GFM
        | Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
}

pub fn analyze(source: &str) -> MarkdownAnalysis {
    let mut headings = Vec::new();
    let mut current_heading: Option<(HeadingLevel, usize, String)> = None;

    for (event, range) in Parser::new_ext(source, parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                current_heading = Some((level, range.start, String::new()));
            }
            Event::Text(text) | Event::Code(text) if current_heading.is_some() => {
                if let Some((_, _, heading_text)) = current_heading.as_mut() {
                    heading_text.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak if current_heading.is_some() => {
                if let Some((_, _, heading_text)) = current_heading.as_mut() {
                    heading_text.push(' ');
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, offset, text)) = current_heading.take() {
                    headings.push(Heading {
                        level: heading_level(level),
                        text,
                        line: source[..offset]
                            .bytes()
                            .filter(|byte| *byte == b'\n')
                            .count()
                            + 1,
                    });
                }
            }
            _ => {}
        }
    }

    MarkdownAnalysis {
        headings,
        characters: source
            .chars()
            .filter(|character| !character.is_whitespace())
            .count(),
        words: source.split_whitespace().count(),
        lines: if source.is_empty() {
            1
        } else {
            source.bytes().filter(|byte| *byte == b'\n').count() + 1
        },
    }
}

pub fn render_html_fragment(source: &str) -> String {
    let parser = Parser::new_ext(source, parser_options());
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}

pub fn local_link_destinations(source: &str) -> Vec<String> {
    let mut destinations = Parser::new_ext(source, parser_options())
        .filter_map(|event| match event {
            Event::Start(Tag::Link { dest_url, .. }) => Some(dest_url.into_string()),
            _ => None,
        })
        .filter(|destination| is_local_link(destination))
        .collect::<Vec<_>>();
    destinations.sort();
    destinations.dedup();
    destinations
}

pub fn blocks(source: &str) -> Vec<MarkdownBlock> {
    BlockIndex::new(source).blocks
}

fn block_ranges(source: &str) -> Vec<Range<usize>> {
    if source.is_empty() {
        return std::iter::once(0..0).collect();
    }

    let mut ranges = Vec::<Range<usize>>::new();
    let mut depth = 0usize;
    let mut block_start = None;

    for (event, range) in Parser::new_ext(source, parser_options()).into_offset_iter() {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    block_start = Some(range.start);
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    ranges.push(block_start.take().unwrap_or(range.start)..range.end);
                }
            }
            _ if depth == 0 => ranges.push(range),
            _ => {}
        }
    }

    if let Some(start) = block_start {
        ranges.push(start..source.len());
    }
    if ranges.is_empty() {
        ranges.push(0..source.len());
    }

    let mut merged = Vec::<Range<usize>>::new();
    for mut range in ranges {
        while range.end > range.start && matches!(source.as_bytes()[range.end - 1], b'\n' | b'\r') {
            range.end -= 1;
        }
        if let Some(previous) = merged.last_mut()
            && range.start < previous.end
        {
            previous.end = previous.end.max(range.end);
            continue;
        }
        merged.push(range);
    }

    merged
}

fn block_from_range(source: &str, id: BlockId, range: Range<usize>) -> MarkdownBlock {
    MarkdownBlock {
        id,
        line: source[..range.start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1,
        range,
    }
}

fn block_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn reconcile_changed_gaps(
    old_source: &str,
    old_blocks: &[MarkdownBlock],
    new_source: &str,
    new_ranges: &[Range<usize>],
    old_used: &mut [bool],
    assigned: &mut [Option<BlockId>],
) {
    let anchors = assigned
        .iter()
        .enumerate()
        .filter_map(|(new_index, id)| {
            id.and_then(|id| {
                old_blocks
                    .iter()
                    .position(|block| block.id == id)
                    .map(|old_index| (old_index, new_index))
            })
        })
        .collect::<Vec<_>>();

    let mut previous_old = 0usize;
    let mut previous_new = 0usize;
    for (next_old, next_new) in anchors
        .into_iter()
        .chain(std::iter::once((old_blocks.len(), new_ranges.len())))
    {
        match_changed_gap(
            old_source,
            old_blocks,
            previous_old..next_old,
            new_source,
            new_ranges,
            previous_new..next_new,
            old_used,
            assigned,
        );
        previous_old = next_old.saturating_add(1);
        previous_new = next_new.saturating_add(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn match_changed_gap(
    old_source: &str,
    old_blocks: &[MarkdownBlock],
    old_gap: Range<usize>,
    new_source: &str,
    new_ranges: &[Range<usize>],
    new_gap: Range<usize>,
    old_used: &mut [bool],
    assigned: &mut [Option<BlockId>],
) {
    let old_indices = old_gap
        .filter(|index| !old_used[*index])
        .collect::<Vec<_>>();
    let new_indices = new_gap
        .filter(|index| assigned[*index].is_none())
        .collect::<Vec<_>>();
    if old_indices.is_empty() || new_indices.is_empty() {
        return;
    }

    if old_indices.len() == new_indices.len() {
        for (old_index, new_index) in old_indices.into_iter().zip(new_indices) {
            assigned[new_index] = Some(old_blocks[old_index].id);
            old_used[old_index] = true;
        }
        return;
    }

    let mut candidates = old_indices
        .iter()
        .flat_map(|old_index| {
            new_indices.iter().map(move |new_index| {
                let old = &old_source[old_blocks[*old_index].range.clone()];
                let new = &new_source[new_ranges[*new_index].clone()];
                (
                    similarity_score(old, new),
                    old_index.abs_diff(*new_index),
                    *old_index,
                    *new_index,
                )
            })
        })
        .filter(|(score, _, _, _)| *score > 0)
        .collect::<Vec<_>>();
    candidates
        .sort_unstable_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    for (_, _, old_index, new_index) in candidates {
        if !old_used[old_index] && assigned[new_index].is_none() {
            assigned[new_index] = Some(old_blocks[old_index].id);
            old_used[old_index] = true;
        }
    }
}

fn similarity_score(left: &str, right: &str) -> usize {
    let prefix = left
        .chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = left
        .chars()
        .rev()
        .zip(right.chars().rev())
        .take_while(|(left, right)| left == right)
        .count();
    prefix + suffix
}

pub fn render_html_document(source: &str, title: &str, dark: bool) -> String {
    let body = render_html_fragment(source);
    let (background, foreground, muted, code_background) = if dark {
        ("#111318", "#e8eaf0", "#a8adba", "#20242c")
    } else {
        ("#ffffff", "#202124", "#69707d", "#f3f4f6")
    };

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{}</title>
  <style>
    :root {{ color-scheme: {}; }}
    body {{
      max-width: 860px; margin: 0 auto; padding: 48px 28px 80px;
      background: {}; color: {}; line-height: 1.7;
      font-family: system-ui, -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif;
    }}
    h1, h2 {{ border-bottom: 1px solid {}; padding-bottom: .3em; }}
    a {{ color: #4d7cff; }}
    blockquote {{ margin-left: 0; padding-left: 1em; border-left: 4px solid #7aa2f7; color: {}; }}
    code {{ background: {}; border-radius: 4px; padding: .15em .35em; }}
    pre {{ background: {}; border-radius: 10px; padding: 16px; overflow: auto; }}
    pre code {{ padding: 0; }}
    table {{ border-collapse: collapse; width: 100%; }}
    th, td {{ border: 1px solid {}; padding: 8px 12px; text-align: left; }}
    img {{ max-width: 100%; }}
  </style>
</head>
<body>{}</body>
</html>
"#,
        escape_html(title),
        if dark { "dark" } else { "light" },
        background,
        foreground,
        muted,
        muted,
        code_background,
        code_background,
        muted,
        body
    )
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

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn is_local_link(destination: &str) -> bool {
    if destination.is_empty() || destination.starts_with('#') {
        return false;
    }
    let before_slash = destination
        .find(['/', '\\', '#', '?'])
        .map_or(destination, |index| &destination[..index]);
    !before_slash.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_atx_and_setext_headings() {
        let analysis = analyze("# First\n\nSecond\n------\n\n### 中文");
        assert_eq!(
            analysis.headings,
            vec![
                Heading {
                    level: 1,
                    text: "First".to_owned(),
                    line: 1,
                },
                Heading {
                    level: 2,
                    text: "Second".to_owned(),
                    line: 3,
                },
                Heading {
                    level: 3,
                    text: "中文".to_owned(),
                    line: 6,
                },
            ]
        );
    }

    #[test]
    fn renders_gfm_table_and_task_list() {
        let html = render_html_fragment("| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done");
        assert!(html.contains("<table>"));
        assert!(html.contains("type=\"checkbox\""));
    }

    #[test]
    fn collects_only_relative_document_links() {
        let links = local_link_destinations(
            "[local](notes/today.md) [anchor](#part) [web](https://example.com) \
             [mail](mailto:test@example.com) [local anchor](other.md#section)",
        );
        assert_eq!(links, vec!["notes/today.md", "other.md#section"]);
    }

    #[test]
    fn splits_markdown_into_top_level_editing_blocks() {
        let source =
            "# Heading\n\nParagraph with **bold**.\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n";
        let blocks = blocks(source);
        let contents = blocks
            .iter()
            .map(|block| &source[block.range.clone()])
            .collect::<Vec<_>>();

        assert_eq!(
            contents,
            vec![
                "# Heading",
                "Paragraph with **bold**.",
                "- one\n- two",
                "```rust\nfn main() {}\n```",
            ]
        );
        assert_eq!(
            blocks.iter().map(|block| block.line).collect::<Vec<_>>(),
            vec![1, 3, 5, 8]
        );
    }

    #[test]
    fn returns_an_editable_block_for_an_empty_document() {
        let blocks = blocks("");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].range, 0..0);
        assert_eq!(blocks[0].line, 1);
    }

    #[test]
    fn stable_block_ids_survive_content_inserted_before_them() {
        let original = "# Heading\n\nFirst paragraph.\n\nSecond paragraph.";
        let mut index = BlockIndex::new(original);
        let original_ids = index
            .blocks()
            .iter()
            .map(|block| block.id)
            .collect::<Vec<_>>();

        let updated = "New introduction.\n\n# Heading\n\nFirst paragraph.\n\nSecond paragraph.";
        index.update(updated);
        let updated_ids = index
            .blocks()
            .iter()
            .map(|block| block.id)
            .collect::<Vec<_>>();

        assert_eq!(&updated_ids[1..], original_ids);
        assert_ne!(updated_ids[0], original_ids[0]);
    }

    #[test]
    fn edited_block_keeps_its_identity_between_unchanged_anchors() {
        let original = "# Heading\n\nOriginal paragraph.\n\n## End";
        let mut index = BlockIndex::new(original);
        let paragraph_id = index.blocks()[1].id;

        index.update("# Heading\n\nChanged paragraph with 中文.\n\n## End");

        assert_eq!(index.blocks()[1].id, paragraph_id);
    }

    #[test]
    fn inserting_a_block_preserves_surrounding_identities() {
        let original = "Alpha.\n\nOmega.";
        let mut index = BlockIndex::new(original);
        let alpha = index.blocks()[0].id;
        let omega = index.blocks()[1].id;

        index.update("Alpha.\n\nInserted.\n\nOmega.");

        assert_eq!(index.blocks()[0].id, alpha);
        assert_eq!(index.blocks()[2].id, omega);
        assert_ne!(index.blocks()[1].id, alpha);
        assert_ne!(index.blocks()[1].id, omega);
    }
}
