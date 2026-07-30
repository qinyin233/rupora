use std::ops::Range;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownBlock {
    pub range: Range<usize>,
    pub line: usize,
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
    if source.is_empty() {
        return vec![MarkdownBlock {
            range: 0..0,
            line: 1,
        }];
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
        .into_iter()
        .map(|range| MarkdownBlock {
            line: source[..range.start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1,
            range,
        })
        .collect()
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
        assert_eq!(
            blocks(""),
            vec![MarkdownBlock {
                range: 0..0,
                line: 1,
            }]
        );
    }
}
