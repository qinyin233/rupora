use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
    path::Path,
    sync::{Arc, OnceLock},
};

use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html,
};
use sha2::{Digest as _, Sha256};

const MAX_MATH_BYTES: usize = 16 * 1024;
const MAX_MERMAID_BYTES: usize = 256 * 1024;
const MAX_MERMAID_LINES: usize = 2_048;
const MAX_GENERATED_SVG_BYTES: usize = 8 * 1024 * 1024;
const MAX_GENERATED_SVG_EDGE: f32 = 8_192.0;
const MAX_GENERATED_SVG_PIXELS: f64 = 16.0 * 1024.0 * 1024.0;
const MAX_GENERATED_SVG_ASPECT_RATIO: f32 = 512.0;
const MAX_GENERATED_BLOCKS: usize = 128;
/// Combined budget for generated math/diagram fragments in one HTML document.
pub const MAX_GENERATED_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOC_BYTES: usize = 1024 * 1024;
const MAX_TOC_EXPANSION_BYTES: usize = 4 * 1024 * 1024;
const MAX_BLOCK_MATCH_CANDIDATES: usize = 16 * 1024;
const MAX_SIMILARITY_CHARS_PER_SIDE: usize = 256;
const TOC_LIMIT_MARKDOWN: &str = "> Table of contents omitted: output budget exceeded.\n";
const TOC_TRUNCATED_MARKDOWN: &str = "> Table of contents truncated: output budget exceeded.\n";
const GENERATED_LIMIT_HTML: &str = "<span class=\"diagram-error\">生成内容超过文档资源预算</span>";

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
pub struct HeadingAnchor {
    pub heading: Heading,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontMatter {
    pub fields: Vec<(String, String)>,
    pub raw: String,
    pub body_start: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MermaidBlock {
    pub range: Range<usize>,
    pub source: String,
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
    references: std::sync::Arc<ReferenceDefinitions>,
    blocks: Vec<MarkdownBlock>,
    next_id: u64,
}

impl BlockIndex {
    pub fn new(source: &str) -> Self {
        let mut next_id = 1;
        let mut previous_start = 0usize;
        let mut line = 1usize;
        let blocks = block_ranges(source)
            .into_iter()
            .map(|range| {
                let id = BlockId(next_id);
                next_id += 1;
                line += source[previous_start..range.start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count();
                previous_start = range.start;
                MarkdownBlock { id, range, line }
            })
            .collect();
        Self {
            source: source.to_owned(),
            references: reference_definitions(source),
            blocks,
            next_id,
        }
    }

    pub fn blocks(&self) -> &[MarkdownBlock] {
        &self.blocks
    }

    pub fn references(&self) -> std::sync::Arc<ReferenceDefinitions> {
        self.references.clone()
    }

    pub fn update(&mut self, source: &str) {
        if self.source == source {
            return;
        }

        self.references = reference_definitions(source);

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
        let mut previous_start = 0usize;
        let mut line = 1usize;
        self.blocks = new_ranges
            .into_iter()
            .enumerate()
            .map(|(index, range)| {
                let id = assigned[index].unwrap_or_else(|| {
                    let id = BlockId(next_id);
                    next_id += 1;
                    id
                });
                line += source[previous_start..range.start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count();
                previous_start = range.start;
                MarkdownBlock { id, range, line }
            })
            .collect();
        self.next_id = next_id;
        self.source.clear();
        self.source.push_str(source);
    }
}

/// Document-wide link definitions shared by block previews and edit projections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferenceDefinitions(HashMap<unicase::UniCase<String>, (String, String)>);

pub fn reference_definitions(source: &str) -> std::sync::Arc<ReferenceDefinitions> {
    let source = parse_front_matter(source).map_or(source, |front| &source[front.body_start..]);
    let parser = Parser::new_ext(source, parser_options());
    std::sync::Arc::new(ReferenceDefinitions(
        parser
            .reference_definitions()
            .iter()
            .map(|(label, definition)| {
                (
                    unicase::UniCase::new(label.to_owned()),
                    (
                        definition.dest.to_string(),
                        definition
                            .title
                            .as_ref()
                            .map_or_else(String::new, ToString::to_string),
                    ),
                )
            })
            .collect(),
    ))
}

pub fn events_with_references<'a>(
    source: &'a str,
    references: &'a ReferenceDefinitions,
) -> impl Iterator<Item = (Event<'a>, Range<usize>)> + 'a {
    Parser::new_with_broken_link_callback(
        source,
        parser_options(),
        Some(move |link: pulldown_cmark::BrokenLink<'_>| {
            references
                .0
                .get(&unicase::UniCase::new(link.reference.to_string()))
                .map(|(destination, title)| (destination.clone().into(), title.clone().into()))
        }),
    )
    .into_offset_iter()
}

pub fn parser_options() -> Options {
    Options::ENABLE_GFM
        | Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
}

pub fn analyze(source: &str) -> MarkdownAnalysis {
    let mut headings = Vec::new();
    let mut current_heading: Option<(HeadingLevel, usize, String)> = None;
    let mut line_scan_offset = 0usize;
    let (body, mut line_at_scan_offset) = parse_front_matter(source).map_or((source, 1), |front| {
        (
            &source[front.body_start..],
            source[..front.body_start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1,
        )
    });

    for (event, range) in Parser::new_ext(body, parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                debug_assert!(range.start >= line_scan_offset);
                line_at_scan_offset += body.as_bytes()[line_scan_offset..range.start]
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count();
                line_scan_offset = range.start;
                current_heading = Some((level, line_at_scan_offset, String::new()));
            }
            Event::Text(text)
            | Event::Code(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
                if current_heading.is_some() =>
            {
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
                if let Some((level, line, text)) = current_heading.take() {
                    headings.push(Heading {
                        level: heading_level(level),
                        text,
                        line,
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

pub fn heading_anchors(source: &str) -> Vec<HeadingAnchor> {
    let mut occurrences = HashMap::<String, usize>::new();
    let headings = analyze(source).headings;
    let body = parse_front_matter(source).map_or(source, |front| &source[front.body_start..]);
    let explicit_ids = Parser::new_ext(body, parser_options())
        .filter_map(|event| match event {
            Event::Start(Tag::Heading { id, .. }) => Some(id.map(CowStr::into_string)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let reserved = explicit_ids
        .iter()
        .flatten()
        .cloned()
        .collect::<HashSet<_>>();
    let mut used = HashSet::new();
    headings
        .into_iter()
        .zip(explicit_ids)
        .map(|(heading, explicit_id)| {
            let base = explicit_id
                .clone()
                .unwrap_or_else(|| heading_slug(&heading.text));
            let mut id = base.clone();
            if used.contains(&id) || (explicit_id.is_none() && reserved.contains(&id)) {
                let occurrence = occurrences.entry(base.clone()).or_default();
                loop {
                    *occurrence += 1;
                    id = format!("{base}-{occurrence}");
                    if !used.contains(&id) && !reserved.contains(&id) {
                        break;
                    }
                }
            }
            used.insert(id.clone());
            HeadingAnchor { heading, id }
        })
        .collect()
}

pub fn parse_front_matter(source: &str) -> Option<FrontMatter> {
    if !source.starts_with("---\n") {
        return None;
    }

    let mut cursor = 4usize;
    for line in source[4..].split_inclusive('\n') {
        let line_start = cursor;
        cursor += line.len();
        if matches!(line.trim(), "---" | "...") {
            let raw = source[4..line_start].trim_end_matches('\n').to_owned();
            let fields = match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&raw) {
                Ok(serde_yaml_ng::Value::Mapping(mapping)) => mapping
                    .into_iter()
                    .map(|(key, value)| (yaml_value_text(&key), yaml_value_text(&value)))
                    .collect(),
                Ok(value) => vec![("value".to_owned(), yaml_value_text(&value))],
                Err(error) => vec![("解析错误".to_owned(), error.to_string())],
            };
            return Some(FrontMatter {
                fields,
                raw,
                body_start: cursor,
            });
        }
    }
    None
}

pub fn mermaid_blocks(source: &str) -> Vec<MermaidBlock> {
    let mut blocks = Vec::new();
    let mut active: Option<(usize, String)> = None;
    for (event, range) in Parser::new_ext(source, parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if info
                    .split_whitespace()
                    .next()
                    .is_some_and(|language| language.eq_ignore_ascii_case("mermaid")) =>
            {
                active = Some((range.start, String::new()));
            }
            Event::Text(text) if active.is_some() => {
                if let Some((_, diagram)) = active.as_mut() {
                    diagram.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) if active.is_some() => {
                if let Some((start, diagram)) = active.take() {
                    blocks.push(MermaidBlock {
                        range: start..range.end,
                        source: diagram,
                    });
                }
            }
            _ => {}
        }
    }
    blocks
}

pub fn prepare_preview_markdown(source: &str) -> String {
    expand_front_matter_and_toc(source)
}

pub fn front_matter_preview_markdown(front_matter: &FrontMatter) -> String {
    let mut output = String::from("> **文档元数据**\n");
    for (key, value) in &front_matter.fields {
        output.push_str("> - **");
        output.push_str(&key.replace(['*', '[', ']'], ""));
        output.push_str("：** ");
        output.push_str(&value.replace('\n', " "));
        output.push('\n');
    }
    output
}

pub fn toc_preview_markdown(source: &str) -> String {
    render_toc_markdown(&heading_anchors(source))
}

pub fn is_toc_marker(source: &str, range: Range<usize>) -> bool {
    source
        .get(range.clone())
        .is_some_and(|text| text.trim().eq_ignore_ascii_case("[TOC]"))
        && !Parser::new_ext(source, parser_options())
            .into_offset_iter()
            .any(|(event, code)| {
                matches!(event, Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock))
                    && code.start < range.end
                    && range.start < code.end
            })
}

pub fn render_math_svg(source: &str, inline: bool) -> Result<String, String> {
    if source.len() > MAX_MATH_BYTES {
        return Err(format!("公式超过 {} KiB 渲染上限", MAX_MATH_BYTES / 1024));
    }
    let nodes = ratex_parser::parse(source).map_err(|error| error.to_string())?;
    let layout = ratex_layout::layout(&nodes, &ratex_layout::LayoutOptions::default());
    let display_list = ratex_layout::to_display_list(&layout);
    let options = ratex_svg::SvgOptions {
        font_size: if inline { 24.0 } else { 34.0 },
        padding: if inline { 2.0 } else { 8.0 },
        embed_glyphs: true,
        ..ratex_svg::SvgOptions::default()
    };
    bound_generated_svg(ratex_svg::render_to_svg(&display_list, &options), "公式")
}

pub fn render_mermaid_svg(source: &str, dark: bool) -> Result<String, String> {
    if source.len() > MAX_MERMAID_BYTES {
        return Err(format!(
            "Mermaid 源码超过 {} KiB 渲染上限",
            MAX_MERMAID_BYTES / 1024
        ));
    }
    if source.lines().count() > MAX_MERMAID_LINES {
        return Err(format!("Mermaid 超过 {MAX_MERMAID_LINES} 行渲染上限"));
    }
    let theme = if dark {
        mermaid_svg::Theme::dark()
    } else {
        mermaid_svg::Theme::default()
    };
    let svg = mermaid_svg::render_with(source, &theme).map_err(|error| error.to_string())?;
    bound_generated_svg(svg, "Mermaid")
}

pub fn render_html_fragment(source: &str) -> String {
    let (mut output, token_prefix, generated) = render_html_with_generated(source, false);
    output = sanitize_user_html(&output);
    restore_generated_html(&output, &token_prefix, &generated)
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

pub fn link_destination_at(source: &str, source_byte: usize) -> Option<String> {
    link_destination_with_references(source, source_byte, &Default::default())
}

pub fn link_destination_with_references(
    source: &str,
    source_byte: usize,
    references: &ReferenceDefinitions,
) -> Option<String> {
    events_with_references(source, references).find_map(|(event, range)| match event {
        Event::Start(Tag::Link { dest_url, .. })
            if range.start <= source_byte && source_byte <= range.end =>
        {
            Some(dest_url.into_string())
        }
        _ => None,
    })
}

pub fn toggle_task_marker_at(source: &str, source_byte: usize) -> Option<String> {
    let (range, checked) = task_markers(source).into_iter().find(|(range, _)| {
        let line_start = source[..range.start]
            .rfind(['\n', '\r'])
            .map_or(0, |index| index + 1);
        line_start <= source_byte && source_byte <= range.end
    })?;
    let mut output = source.to_owned();
    output.replace_range(range, if checked { "[ ]" } else { "[x]" });
    Some(output)
}

pub fn is_local_link_destination(destination: &str) -> bool {
    is_local_link(destination)
}

pub fn local_image_destinations(source: &str) -> Vec<String> {
    let mut destinations = Parser::new_ext(source, parser_options())
        .filter_map(|event| match event {
            Event::Start(Tag::Image { dest_url, .. }) => Some(dest_url.into_string()),
            _ => None,
        })
        .filter(|destination| is_local_image_destination(destination))
        .collect::<Vec<_>>();
    destinations.sort();
    destinations.dedup();
    destinations
}

fn is_local_image_destination(destination: &str) -> bool {
    if is_local_link(destination) {
        return true;
    }
    let path = destination.split(['?', '#']).next().unwrap_or_default();
    let bytes = path.as_bytes();
    let windows_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    Path::new(path).is_absolute() || windows_absolute
}

pub fn synchronize_task_markers(source: &str, rendered_markdown: &str) -> Option<String> {
    let source_markers = task_markers(source);
    let rendered_states = task_markers(rendered_markdown)
        .into_iter()
        .map(|(_, checked)| checked)
        .collect::<Vec<_>>();
    if source_markers.len() != rendered_states.len() {
        return None;
    }
    let mut output = source.to_owned();
    let mut changed = false;
    for ((range, before), after) in source_markers.into_iter().zip(rendered_states).rev() {
        if before != after {
            output.replace_range(range, if after { "[x]" } else { "[ ]" });
            changed = true;
        }
    }
    changed.then_some(output)
}

pub fn blocks(source: &str) -> Vec<MarkdownBlock> {
    BlockIndex::new(source).blocks
}

fn task_markers(source: &str) -> Vec<(Range<usize>, bool)> {
    Parser::new_ext(source, parser_options())
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::TaskListMarker(checked) => Some((range, checked)),
            _ => None,
        })
        .collect()
}

fn block_ranges(source: &str) -> Vec<Range<usize>> {
    if source.is_empty() {
        return std::iter::once(0..0).collect();
    }

    let mut ranges = Vec::<Range<usize>>::new();
    let body_start = parse_front_matter(source).map_or(0, |front| front.body_start);
    if body_start > 0 {
        let mut front_matter_end = body_start;
        while front_matter_end > 0
            && matches!(source.as_bytes()[front_matter_end - 1], b'\n' | b'\r')
        {
            front_matter_end -= 1;
        }
        ranges.push(0..front_matter_end);
    }
    let body = &source[body_start..];
    let mut depth = 0usize;
    let mut block_start = None;

    for (event, range) in Parser::new_ext(body, parser_options()).into_offset_iter() {
        let range = range.start + body_start..range.end + body_start;
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

    editable_paragraph_ranges(source, merged)
}

/// Empty paragraphs need their own cursor targets. They are omitted by the
/// Markdown parser but must not appear only after the user starts typing.
fn editable_paragraph_ranges(source: &str, mut blocks: Vec<Range<usize>>) -> Vec<Range<usize>> {
    fn breaks(source: &str, range: Range<usize>) -> impl Iterator<Item = usize> + '_ {
        source[range.clone()]
            .char_indices()
            .filter_map(move |(offset, character)| {
                (character == '\n'
                    || character == '\r'
                        && !source[range.start + offset + 1..range.end].starts_with('\n'))
                .then_some(range.start + offset + 1)
            })
    }
    if source.trim().is_empty() {
        let mut ranges = std::iter::once(0..0).collect::<Vec<_>>();
        ranges.extend(
            breaks(source, 0..source.len())
                .skip(1)
                .step_by(2)
                .map(|at| at..at),
        );
        for range in &mut ranges {
            range.end = source[range.start..]
                .find(['\r', '\n'])
                .map_or(source.len(), |at| range.start + at);
        }
        return ranges;
    }
    // Retain explicit hard-break rows at a paragraph's end. Their spaces are
    // Markdown syntax, including when the following visible line is empty.
    for index in 0..blocks.len() {
        let limit = blocks
            .get(index + 1)
            .map_or(source.len(), |next| next.start);
        let mut end = blocks[index].end;
        while source[..end].ends_with("  ") || source[..end].ends_with('\\') {
            let Some(newline) = breaks(source, end..limit).next() else {
                break;
            };
            if !source[end..newline].chars().all(char::is_whitespace) {
                break;
            }
            end = newline;
            let spaces = source[end..limit]
                .bytes()
                .take_while(|byte| *byte == b' ')
                .count();
            if spaces >= 2 {
                end += spaces;
            } else {
                break;
            }
        }
        blocks[index].end = end;
    }
    let mut result = Vec::new();
    let mut previous_end = 0;
    for (index, block) in blocks.into_iter().enumerate() {
        let gap = previous_end..block.start;
        if source[gap.clone()].chars().all(char::is_whitespace) {
            let lines = breaks(source, gap).collect::<Vec<_>>();
            if index == 0 && !lines.is_empty() {
                result.push(0..0);
            }
            result.extend(
                lines
                    .into_iter()
                    .skip(1)
                    .step_by(2)
                    .filter(|at| *at < block.start)
                    .map(|at| at..at),
            );
        }
        previous_end = block.end;
        result.push(block);
    }
    let tail = previous_end..source.len();
    if source[tail.clone()].chars().all(char::is_whitespace) {
        result.extend(breaks(source, tail).skip(1).step_by(2).map(|at| at..at));
    }
    result
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
    let old_positions = old_blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.id, index))
        .collect::<HashMap<_, _>>();
    let anchors = assigned
        .iter()
        .enumerate()
        .filter_map(|(new_index, id)| {
            id.and_then(|id| {
                old_positions
                    .get(&id)
                    .copied()
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

    if old_indices.len().min(new_indices.len()) > MAX_BLOCK_MATCH_CANDIDATES {
        match_changed_gap_positionally(
            old_source,
            old_blocks,
            &old_indices,
            new_source,
            new_ranges,
            &new_indices,
            old_used,
            assigned,
        );
        return;
    }

    let mut candidates = Vec::new();
    if old_indices
        .len()
        .checked_mul(new_indices.len())
        .is_some_and(|count| count <= MAX_BLOCK_MATCH_CANDIDATES)
    {
        for &old_index in &old_indices {
            for &new_index in &new_indices {
                push_block_match_candidate(
                    old_source,
                    old_blocks,
                    old_index,
                    new_source,
                    new_ranges,
                    new_index,
                    &mut candidates,
                );
            }
        }
    } else {
        collect_bounded_block_match_candidates(
            old_source,
            old_blocks,
            &old_indices,
            new_source,
            new_ranges,
            &new_indices,
            &mut candidates,
        );
    }
    candidates.sort_unstable_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.cmp(&right.3))
    });

    for (_, _, old_index, new_index) in candidates {
        if !old_used[old_index] && assigned[new_index].is_none() {
            assigned[new_index] = Some(old_blocks[old_index].id);
            old_used[old_index] = true;
        }
    }
}

type BlockMatchCandidate = (usize, usize, usize, usize);

#[allow(clippy::too_many_arguments)]
fn push_block_match_candidate(
    old_source: &str,
    old_blocks: &[MarkdownBlock],
    old_index: usize,
    new_source: &str,
    new_ranges: &[Range<usize>],
    new_index: usize,
    candidates: &mut Vec<BlockMatchCandidate>,
) {
    let old = &old_source[old_blocks[old_index].range.clone()];
    let new = &new_source[new_ranges[new_index].clone()];
    let score = similarity_score(old, new);
    if score > 0 {
        candidates.push((score, old_index.abs_diff(new_index), old_index, new_index));
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_bounded_block_match_candidates(
    old_source: &str,
    old_blocks: &[MarkdownBlock],
    old_indices: &[usize],
    new_source: &str,
    new_ranges: &[Range<usize>],
    new_indices: &[usize],
    candidates: &mut Vec<BlockMatchCandidate>,
) {
    let smaller_len = old_indices.len().min(new_indices.len());
    let slots_per_item = (MAX_BLOCK_MATCH_CANDIDATES / smaller_len).max(1);
    let center_count = if slots_per_item >= 3 { 3 } else { 1 };
    let window_len = (slots_per_item / center_count).max(1);

    if old_indices.len() <= new_indices.len() {
        for (old_position, &old_index) in old_indices.iter().enumerate() {
            let centers = alignment_centers(old_position, old_indices.len(), new_indices.len());
            for &center in centers.iter().take(center_count) {
                for new_position in centered_window(center, new_indices.len(), window_len) {
                    push_block_match_candidate(
                        old_source,
                        old_blocks,
                        old_index,
                        new_source,
                        new_ranges,
                        new_indices[new_position],
                        candidates,
                    );
                }
            }
        }
    } else {
        for (new_position, &new_index) in new_indices.iter().enumerate() {
            let centers = alignment_centers(new_position, new_indices.len(), old_indices.len());
            for &center in centers.iter().take(center_count) {
                for old_position in centered_window(center, old_indices.len(), window_len) {
                    push_block_match_candidate(
                        old_source,
                        old_blocks,
                        old_indices[old_position],
                        new_source,
                        new_ranges,
                        new_index,
                        candidates,
                    );
                }
            }
        }
    }

    debug_assert!(candidates.len() <= MAX_BLOCK_MATCH_CANDIDATES);
}

#[allow(clippy::too_many_arguments)]
fn match_changed_gap_positionally(
    old_source: &str,
    old_blocks: &[MarkdownBlock],
    old_indices: &[usize],
    new_source: &str,
    new_ranges: &[Range<usize>],
    new_indices: &[usize],
    old_used: &mut [bool],
    assigned: &mut [Option<BlockId>],
) {
    if old_indices.len() <= new_indices.len() {
        for (position, &old_index) in old_indices.iter().enumerate() {
            let new_position = scaled_position(position, old_indices.len(), new_indices.len());
            let new_index = new_indices[new_position];
            let old = &old_source[old_blocks[old_index].range.clone()];
            let new = &new_source[new_ranges[new_index].clone()];
            if similarity_score(old, new) > 0 {
                assigned[new_index] = Some(old_blocks[old_index].id);
                old_used[old_index] = true;
            }
        }
    } else {
        for (position, &new_index) in new_indices.iter().enumerate() {
            let old_position = scaled_position(position, new_indices.len(), old_indices.len());
            let old_index = old_indices[old_position];
            let old = &old_source[old_blocks[old_index].range.clone()];
            let new = &new_source[new_ranges[new_index].clone()];
            if similarity_score(old, new) > 0 {
                assigned[new_index] = Some(old_blocks[old_index].id);
                old_used[old_index] = true;
            }
        }
    }
}

fn alignment_centers(position: usize, smaller_len: usize, larger_len: usize) -> [usize; 3] {
    [
        scaled_position(position, smaller_len, larger_len),
        position,
        larger_len - smaller_len + position,
    ]
}

fn scaled_position(position: usize, from_len: usize, to_len: usize) -> usize {
    if from_len <= 1 {
        return 0;
    }
    ((position as u128 * (to_len - 1) as u128) / (from_len - 1) as u128) as usize
}

fn centered_window(center: usize, total_len: usize, requested_len: usize) -> Range<usize> {
    let window_len = requested_len.min(total_len);
    let start = center
        .saturating_sub(window_len / 2)
        .min(total_len - window_len);
    start..start + window_len
}

fn similarity_score(left: &str, right: &str) -> usize {
    let prefix = left
        .chars()
        .zip(right.chars())
        .take(MAX_SIMILARITY_CHARS_PER_SIDE)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = left
        .chars()
        .rev()
        .zip(right.chars().rev())
        .take(MAX_SIMILARITY_CHARS_PER_SIDE)
        .take_while(|(left, right)| left == right)
        .count();
    prefix + suffix
}

fn expand_front_matter_and_toc(source: &str) -> String {
    let (front_matter, body) = if let Some(front_matter) = parse_front_matter(source) {
        let body = &source[front_matter.body_start..];
        (Some(front_matter), body)
    } else {
        (None, source)
    };
    let anchors = heading_anchors(body);
    let toc = render_toc_markdown(&anchors);
    let mut output = String::with_capacity(
        source
            .len()
            .saturating_add(toc.len().min(MAX_TOC_EXPANSION_BYTES)),
    );

    if let Some(front_matter) = front_matter {
        output.push_str(&front_matter_preview_markdown(&front_matter));
        output.push('\n');
    }

    let mut literal_ranges = Parser::new_ext(body, parser_options())
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(event, Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock)).then_some(range)
        })
        .peekable();
    let mut body_offset = 0usize;
    let mut toc_expansion_bytes = 0usize;
    let mut toc_limit_reported = false;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim();
        while literal_ranges
            .peek()
            .is_some_and(|range| range.end <= body_offset)
        {
            literal_ranges.next();
        }
        let in_literal = literal_ranges
            .peek()
            .is_some_and(|range| range.start < body_offset + line.len() && body_offset < range.end);
        if !in_literal && trimmed.eq_ignore_ascii_case("[TOC]") {
            if toc_expansion_bytes
                .checked_add(toc.len())
                .is_some_and(|bytes| bytes <= MAX_TOC_EXPANSION_BYTES)
            {
                output.push_str(&toc);
                toc_expansion_bytes += toc.len();
                if line.ends_with('\n') && !toc.ends_with('\n') {
                    output.push('\n');
                }
            } else if !toc_limit_reported {
                output.push_str(TOC_LIMIT_MARKDOWN);
                toc_limit_reported = true;
            } else {
                output.push_str(line);
            }
        } else {
            output.push_str(line);
        }
        body_offset += line.len();
    }
    if !body.is_empty() && !body.ends_with('\n') && output.is_empty() {
        output.push_str(body);
    }
    output
}

fn render_toc_markdown(anchors: &[HeadingAnchor]) -> String {
    if anchors.is_empty() {
        return "> 文档暂无可用标题。\n".to_owned();
    }
    let minimum_level = anchors
        .iter()
        .map(|anchor| anchor.heading.level)
        .min()
        .unwrap_or(1);
    let mut output = String::with_capacity(MAX_TOC_BYTES.min(anchors.len().saturating_mul(64)));
    for anchor in anchors {
        let indent = anchor.heading.level.saturating_sub(minimum_level) as usize;
        let escaped_text_bytes = anchor
            .heading
            .text
            .chars()
            .map(|character| character.len_utf8() + usize::from(character.is_ascii_punctuation()))
            .fold(0usize, usize::saturating_add);
        let entry_bytes = indent
            .saturating_mul(2)
            .saturating_add(8)
            .saturating_add(escaped_text_bytes)
            .saturating_add(
                anchor
                    .id
                    .chars()
                    .map(|character| {
                        character.len_utf8()
                            + if matches!(character, '%' | '#') {
                                2
                            } else {
                                usize::from(toc_fragment_needs_markdown_escape(character))
                            }
                    })
                    .fold(0usize, usize::saturating_add),
            );
        if output
            .len()
            .saturating_add(entry_bytes)
            .saturating_add(TOC_TRUNCATED_MARKDOWN.len())
            > MAX_TOC_BYTES
        {
            output.push_str(TOC_TRUNCATED_MARKDOWN);
            break;
        }

        for _ in 0..indent {
            output.push_str("  ");
        }
        output.push_str("- [");
        for character in anchor.heading.text.chars() {
            if character.is_ascii_punctuation() {
                output.push('\\');
            }
            output.push(character);
        }
        output.push_str("](#");
        for character in anchor.id.chars() {
            // The Markdown parser consumes punctuation escapes, while URI
            // navigation consumes percent escapes. Preserve the literal ID
            // through both stages, including entities and a literal `%xx`.
            if matches!(character, '%' | '#') {
                output.push_str(if character == '%' { "%25" } else { "%23" });
            } else {
                if toc_fragment_needs_markdown_escape(character) {
                    output.push('\\');
                }
                output.push(character);
            }
        }
        output.push_str(")\n");
    }
    output
}

fn toc_fragment_needs_markdown_escape(character: char) -> bool {
    character.is_ascii_punctuation() && !matches!(character, '-' | '_' | '.' | '~')
}

fn heading_slug(text: &str) -> String {
    let mut output = String::new();
    let mut pending_separator = false;
    for character in text.chars() {
        if character.is_alphanumeric() || matches!(character, '_' | '-') {
            if pending_separator && !output.is_empty() && !output.ends_with('-') {
                output.push('-');
            }
            output.extend(character.to_lowercase());
            pending_separator = false;
        } else if character.is_whitespace() {
            pending_separator = true;
        }
    }
    if output.is_empty() {
        "section".to_owned()
    } else {
        output
    }
}

fn yaml_value_text(value: &serde_yaml_ng::Value) -> String {
    match value {
        serde_yaml_ng::Value::Null => "null".to_owned(),
        serde_yaml_ng::Value::Bool(value) => value.to_string(),
        serde_yaml_ng::Value::Number(value) => value.to_string(),
        serde_yaml_ng::Value::String(value) => value.clone(),
        _ => serde_yaml_ng::to_string(value)
            .unwrap_or_else(|_| format!("{value:?}"))
            .trim()
            .to_owned(),
    }
}

fn render_html_with_generated(source: &str, dark: bool) -> (String, String, Vec<String>) {
    let expanded = expand_front_matter_and_toc(source);
    let anchors = heading_anchors(&expanded);
    let mut heading_index = 0usize;
    let token_prefix = generated_html_token_prefix(&expanded);
    let mut generated = Vec::<String>::new();
    let mut generated_bytes = 0usize;
    let mut events = Vec::<Event<'static>>::new();
    let mut parser = Parser::new_ext(&expanded, parser_options()).into_offset_iter();

    while let Some((event, _)) = parser.next() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if info
                    .split_whitespace()
                    .next()
                    .is_some_and(|language| language.eq_ignore_ascii_case("mermaid")) =>
            {
                let mut diagram = String::new();
                for (event, _) in parser.by_ref() {
                    match event {
                        Event::Text(text) => diagram.push_str(&text),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                if generated.len() >= MAX_GENERATED_BLOCKS
                    || generated_bytes >= MAX_GENERATED_DOCUMENT_BYTES
                {
                    events.push(Event::Html(CowStr::Borrowed(GENERATED_LIMIT_HTML)));
                    continue;
                }
                let rendered = render_mermaid_svg(&diagram, dark)
                    .and_then(|svg| static_svg_for_html(&svg))
                    .unwrap_or_else(|error| {
                        format!(
                            "<pre class=\"diagram-error\">Mermaid：{}</pre>",
                            escape_html(&error)
                        )
                    });
                push_generated_html(
                    &mut events,
                    &mut generated,
                    &mut generated_bytes,
                    &token_prefix,
                    rendered,
                );
            }
            Event::InlineMath(math) => {
                if generated.len() >= MAX_GENERATED_BLOCKS
                    || generated_bytes >= MAX_GENERATED_DOCUMENT_BYTES
                {
                    events.push(Event::Html(CowStr::Borrowed(GENERATED_LIMIT_HTML)));
                    continue;
                }
                let rendered = render_math_svg(&math, true)
                    .and_then(|svg| static_svg_for_html(&svg))
                    .unwrap_or_else(|error| {
                        format!(
                            "<code class=\"math-error\" title=\"{}\">{}</code>",
                            escape_html(&error),
                            escape_html(&math)
                        )
                    });
                let rendered = format!("<span class=\"math-inline\">{rendered}</span>");
                push_generated_html(
                    &mut events,
                    &mut generated,
                    &mut generated_bytes,
                    &token_prefix,
                    rendered,
                );
            }
            Event::DisplayMath(math) => {
                if generated.len() >= MAX_GENERATED_BLOCKS
                    || generated_bytes >= MAX_GENERATED_DOCUMENT_BYTES
                {
                    events.push(Event::Html(CowStr::Borrowed(GENERATED_LIMIT_HTML)));
                    continue;
                }
                let rendered = render_math_svg(&math, false)
                    .and_then(|svg| static_svg_for_html(&svg))
                    .unwrap_or_else(|error| {
                        format!(
                            "<code class=\"math-error\" title=\"{}\">{}</code>",
                            escape_html(&error),
                            escape_html(&math)
                        )
                    });
                let rendered = format!("<div class=\"math-display\">{rendered}</div>");
                push_generated_html(
                    &mut events,
                    &mut generated,
                    &mut generated_bytes,
                    &token_prefix,
                    rendered,
                );
            }
            Event::Start(Tag::Heading {
                level,
                id: _,
                classes,
                attrs,
            }) => {
                let generated_id = anchors
                    .get(heading_index)
                    .map(|anchor| anchor.id.clone())
                    .unwrap_or_else(|| format!("section-{}", heading_index + 1));
                heading_index += 1;
                events.push(Event::Start(
                    Tag::Heading {
                        level,
                        id: Some(CowStr::Boxed(generated_id.into_boxed_str())),
                        classes,
                        attrs,
                    }
                    .into_static(),
                ));
            }
            event => events.push(event.into_static()),
        }
    }

    let mut output = String::new();
    html::push_html(&mut output, events.into_iter());
    (output, token_prefix, generated)
}

fn generated_html_token_prefix(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"rupora-generated-html-token-v2\0");
    hasher.update(source.len().to_le_bytes());
    hasher.update(source.as_bytes());
    format!("RUPORA_GENERATED_BLOCK_{:x}_", hasher.finalize())
}

fn push_generated_html(
    events: &mut Vec<Event<'static>>,
    generated: &mut Vec<String>,
    generated_bytes: &mut usize,
    token_prefix: &str,
    rendered: String,
) {
    let Some(projected_bytes) = generated_bytes.checked_add(rendered.len()) else {
        *generated_bytes = MAX_GENERATED_DOCUMENT_BYTES;
        events.push(Event::Html(CowStr::Borrowed(GENERATED_LIMIT_HTML)));
        return;
    };
    if projected_bytes > MAX_GENERATED_DOCUMENT_BYTES {
        *generated_bytes = MAX_GENERATED_DOCUMENT_BYTES;
        events.push(Event::Html(CowStr::Borrowed(GENERATED_LIMIT_HTML)));
        return;
    }

    let token = format!("{token_prefix}{}__", generated.len());
    generated.push(rendered);
    *generated_bytes = projected_bytes;
    events.push(Event::Html(CowStr::Boxed(token.into_boxed_str())));
}

fn restore_generated_html(body: &str, token_prefix: &str, generated: &[String]) -> String {
    let extra_bytes = generated.iter().map(String::len).sum::<usize>();
    let mut output = String::with_capacity(body.len().saturating_add(extra_bytes));
    let mut remaining = body;

    while let Some(start) = remaining.find(token_prefix) {
        output.push_str(&remaining[..start]);
        let token_body = &remaining[start + token_prefix.len()..];
        let Some(end) = token_body.find("__") else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let Ok(index) = token_body[..end].parse::<usize>() else {
            output.push_str(token_prefix);
            remaining = token_body;
            continue;
        };
        let Some(rendered) = generated.get(index) else {
            output.push_str(token_prefix);
            remaining = token_body;
            continue;
        };
        output.push_str(rendered);
        remaining = &token_body[end + 2..];
    }
    output.push_str(remaining);
    output
}

fn bound_generated_svg(svg: String, kind: &str) -> Result<String, String> {
    if svg.len() > MAX_GENERATED_SVG_BYTES {
        return Err(format!(
            "{kind} SVG 超过 {} MiB 输出上限",
            MAX_GENERATED_SVG_BYTES / 1024 / 1024
        ));
    }
    let tree = parse_generated_svg(&svg)?;
    validate_generated_svg_size(&tree, kind)?;
    Ok(svg)
}

fn generated_svg_options() -> usvg::Options<'static> {
    static FONT_DATABASE: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();

    let font_database = FONT_DATABASE.get_or_init(|| {
        let mut database = usvg::fontdb::Database::new();
        database.load_system_fonts();
        Arc::new(database)
    });
    let mut options = usvg::Options {
        fontdb: Arc::clone(font_database),
        ..usvg::Options::default()
    };
    // Export generated content only as static artwork. Resolving an image from
    // a user-controlled SVG could otherwise expose local or network resources.
    options.image_href_resolver = usvg::ImageHrefResolver {
        resolve_data: Box::new(|_, _, _| None),
        resolve_string: Box::new(|_, _| None),
    };
    options
}

fn parse_generated_svg(svg: &str) -> Result<usvg::Tree, String> {
    usvg::Tree::from_str(svg, &generated_svg_options())
        .map_err(|error| format!("无法安全解析生成的 SVG：{error}"))
}

fn validate_generated_svg_size(tree: &usvg::Tree, kind: &str) -> Result<(), String> {
    let size = tree.size();
    let width = size.width();
    let height = size.height();
    let pixels = f64::from(width) * f64::from(height);
    let aspect_ratio = if width > 0.0 && height > 0.0 {
        width.max(height) / width.min(height)
    } else {
        f32::INFINITY
    };
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > MAX_GENERATED_SVG_EDGE
        || height > MAX_GENERATED_SVG_EDGE
        || pixels > MAX_GENERATED_SVG_PIXELS
        || aspect_ratio > MAX_GENERATED_SVG_ASPECT_RATIO
    {
        return Err(format!(
            "{kind} SVG 尺寸 {width:.0}×{height:.0} 超过预览资源上限"
        ));
    }
    Ok(())
}

fn static_svg_for_html(svg: &str) -> Result<String, String> {
    let tree = parse_generated_svg(svg)?;
    validate_generated_svg_size(&tree, "静态")?;
    let static_svg = tree.to_string(&usvg::WriteOptions::default());
    bound_generated_svg(static_svg, "静态")
}

pub fn render_html_document(source: &str, title: &str, dark: bool) -> String {
    let (unsafe_body, token_prefix, generated) = render_html_with_generated(source, dark);
    let sanitized_body = sanitize_user_html(&unsafe_body);
    let body = restore_generated_html(&sanitized_body, &token_prefix, &generated);
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
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'">
  <title>{}</title>
  <style>
    :root {{ color-scheme: {}; }}
    body {{
      max-width: 860px; margin: 0 auto; padding: 48px 28px 80px;
      background: {}; color: {}; line-height: 1.7;
      font-family: "RUPORA CJK", system-ui, -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif;
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
    svg {{ max-width: 100%; height: auto; }}
    .math-inline svg {{ display: inline-block; width: auto; height: 1.4em; vertical-align: -.35em; }}
    .math-display {{ margin: 1.2em 0; overflow-x: auto; text-align: center; }}
    .math-display svg {{ width: auto; }}
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

fn sanitize_user_html(html: &str) -> String {
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(&["input"])
        .add_generic_attributes(&["id", "class"])
        .add_tag_attributes("input", &["type", "checked", "disabled"]);
    builder.clean(html).to_string()
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
    let bytes = destination.as_bytes();
    let windows_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if destination.is_empty() || destination.starts_with(['#', '/', '\\']) || windows_drive {
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
    fn full_audit_toc_markers_inside_raw_html_remain_literal() {
        for container in ["pre", "script", "style"] {
            let source = format!("<{container}>\n[TOC]\n</{container}>\n\n# Heading\n");
            assert_eq!(
                prepare_preview_markdown(&source),
                source,
                "raw HTML contents must not be rewritten as Markdown TOC entries"
            );
        }
    }

    #[test]
    fn full_audit_toc_preserves_explicit_heading_ids_with_parentheses() {
        let source = "[TOC]\n\n# Heading {#part)}\n";
        let anchors = heading_anchors(source);
        assert_eq!(
            anchors[0].id, "part)",
            "fixture must contain an explicit ID"
        );
        let toc = toc_preview_markdown(source);
        let links = events_with_references(&toc, &Default::default())
            .filter_map(|(event, _)| match event {
                Event::Start(Tag::Link { dest_url, .. }) => Some(dest_url.into_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            links,
            vec!["#part)"],
            "TOC Markdown must preserve the complete target: {toc}"
        );
    }

    #[test]
    fn full_audit_toc_encodes_literal_percent_and_hash_in_explicit_ids() {
        for (id, fragment) in [("%61", "%2561"), ("#part", "%23part")] {
            let source = format!("[TOC]\n\n# Heading {{#{id}}}\n");
            assert_eq!(heading_anchors(&source)[0].id, id);
            let toc = toc_preview_markdown(&source);
            let destination = events_with_references(&toc, &Default::default()).find_map(
                |(event, _)| match event {
                    Event::Start(Tag::Link { dest_url, .. }) => Some(dest_url.into_string()),
                    _ => None,
                },
            );
            assert_eq!(
                destination,
                Some(format!("#{fragment}")),
                "one URI decoding pass must recover the literal heading ID"
            );
            let html = render_html_fragment(&source);
            assert!(html.contains(&format!("href=\"#{fragment}\"")));
            assert!(html.contains(&format!("id=\"{id}\"")));
        }
    }

    #[test]
    fn full_audit_toc_preserves_literal_html_text_from_a_code_heading() {
        let source = "[TOC]\n\n# `<Type>`\n";
        let anchors = heading_anchors(source);
        assert_eq!(anchors[0].heading.text, "<Type>");
        let toc = toc_preview_markdown(source);
        let mut link_text = String::new();
        let mut in_link = false;
        for (event, _) in events_with_references(&toc, &Default::default()) {
            match event {
                Event::Start(Tag::Link { .. }) => in_link = true,
                Event::End(TagEnd::Link) => in_link = false,
                Event::Text(text) | Event::Code(text) if in_link => link_text.push_str(&text),
                _ => {}
            }
        }
        assert_eq!(
            link_text, "<Type>",
            "TOC labels are plain heading text, not HTML: {toc}"
        );
    }

    #[test]
    fn source_audit_toc_on_first_indented_code_line_stays_literal() {
        for indent in ["    ", "\t"] {
            let source = format!("{indent}[TOC]\n\n# Heading\n");
            assert_eq!(prepare_preview_markdown(&source), source);
        }
    }

    #[test]
    fn source_audit_outline_keeps_math_in_heading_labels() {
        let analysis = analyze("# Energy $E=mc^2$\n");
        assert_eq!(analysis.headings[0].text, "Energy E=mc^2");
    }

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
    fn computes_many_heading_lines_in_source_order() {
        let source = (0..8_192)
            .map(|index| format!("# Heading {index}\n\n"))
            .collect::<String>();

        let analysis = analyze(&source);

        assert_eq!(analysis.headings.len(), 8_192);
        assert_eq!(analysis.headings[4_096].line, 8_193);
        assert_eq!(analysis.headings.last().unwrap().line, 16_383);
    }

    #[test]
    fn renders_gfm_table_and_task_list() {
        let html = render_html_fragment("| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done");
        assert!(html.contains("<table>"));
        assert!(html.contains("type=\"checkbox\""));
    }

    #[test]
    fn pulldown_accepts_compact_gfm_table_delimiters() {
        let html = render_html_fragment("| A | B |\n| - | :-: |\n| 1 | 2 |");
        assert!(html.contains("<table>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn synchronizes_preview_task_changes_back_to_the_source() {
        let source = "- [ ] first\n- [x] second\n";
        let rendered = "> metadata\n\n- [x] first\n- [ ] second\n";
        assert_eq!(
            synchronize_task_markers(source, rendered).unwrap(),
            "- [x] first\n- [ ] second\n"
        );
    }

    #[test]
    fn parses_yaml_front_matter_and_hides_it_from_the_document_body() {
        let source = "---\ntitle: Native Rust\ntags: [editor, markdown]\n---\n# Body\n";
        let front_matter = parse_front_matter(source).unwrap();
        assert_eq!(
            front_matter.fields[0],
            ("title".to_owned(), "Native Rust".to_owned())
        );
        assert!(front_matter.body_start > front_matter.raw.len());

        let html = render_html_fragment(source);
        assert!(html.contains("文档元数据"));
        assert!(html.contains("<h1 id=\"body\">Body</h1>"));
        assert!(!html.contains("title: Native Rust"));
    }

    #[test]
    fn front_matter_is_one_native_editing_block() {
        let source = "---\ntitle: Native\ntags: [rust, markdown]\n---\n\n# Body\n";
        let blocks = blocks(source);
        assert_eq!(
            &source[blocks[0].range.clone()],
            "---\ntitle: Native\ntags: [rust, markdown]\n---"
        );
        assert_eq!(&source[blocks[1].range.clone()], "# Body");

        let front = parse_front_matter(source).unwrap();
        let preview = front_matter_preview_markdown(&front);
        assert!(preview.contains("文档元数据"));
        assert!(preview.contains("title"));
        assert!(!preview.contains("---"));
    }

    #[test]
    fn creates_unique_unicode_heading_anchors_and_expands_toc() {
        let source = "[TOC]\n\n# 开始\n\n## Same\n\n## Same\n";
        let anchors = heading_anchors(source);
        assert_eq!(anchors[0].id, "开始");
        assert_eq!(anchors[1].id, "same");
        assert_eq!(anchors[2].id, "same-1");

        let html = render_html_fragment(source);
        assert!(html.contains("href=\"#same-1\""));
        assert!(html.contains("<h2 id=\"same-1\">Same</h2>"));
    }

    #[test]
    fn explicit_heading_ids_drive_toc_export_and_navigation() {
        let source = "[TOC]\n\n# Title {#custom}\n";
        let anchors = heading_anchors(source);
        assert_eq!(anchors[0].id, "custom");

        let html = render_html_fragment(source);
        assert!(html.contains("href=\"#custom\""));
        assert!(html.contains("<h1 id=\"custom\">Title</h1>"));
        assert!(!html.contains("href=\"#title\""));
    }

    #[test]
    fn front_matter_fields_never_leak_into_the_outline() {
        let source = "---\ntitle: Metadata\n---\n\n# Body\n";
        let analysis = analyze(source);
        assert_eq!(analysis.headings.len(), 1);
        assert_eq!(analysis.headings[0].text, "Body");
        assert_eq!(analysis.headings[0].line, 5);
    }

    #[test]
    fn toc_expansion_uses_parsed_code_block_ranges() {
        let source = "    ```\n    [TOC]\n\n[TOC]\n\n# Real heading\n";
        let expanded = expand_front_matter_and_toc(source);

        assert!(expanded.starts_with("    ```\n    [TOC]\n"));
        assert_eq!(expanded.matches("[TOC]").count(), 1);
        assert_eq!(expanded.matches("[Real heading](#real-heading)").count(), 1);

        let fenced = "```text\n[TOC]\n```\n\n[TOC]\n\n# Outside\n";
        let expanded = expand_front_matter_and_toc(fenced);
        assert!(expanded.starts_with("```text\n[TOC]\n```"));
        assert_eq!(expanded.matches("[TOC]").count(), 1);
        assert!(expanded.contains("[Outside](#outside)"));
    }

    #[test]
    fn bounds_large_tables_of_contents_and_repeated_expansions() {
        let anchors = (0..20_000)
            .map(|index| HeadingAnchor {
                heading: Heading {
                    level: 2,
                    text: format!("A deliberately long heading used for budget testing {index}"),
                    line: index + 1,
                },
                id: format!("budget-heading-{index}"),
            })
            .collect::<Vec<_>>();
        let toc = render_toc_markdown(&anchors);
        assert!(toc.len() <= MAX_TOC_BYTES);
        assert!(toc.contains(TOC_TRUNCATED_MARKDOWN.trim()));

        let mut source = "[TOC]\n".repeat(256);
        for index in 0..1_024 {
            source.push_str(&format!("# Repeated table heading number {index}\n\n"));
        }
        let expanded = expand_front_matter_and_toc(&source);
        assert!(expanded.contains(TOC_LIMIT_MARKDOWN.trim()));
        assert!(
            expanded.len()
                <= source
                    .len()
                    .saturating_add(MAX_TOC_EXPANSION_BYTES)
                    .saturating_add(TOC_LIMIT_MARKDOWN.len())
        );
    }

    #[test]
    fn renders_math_and_mermaid_without_a_browser_runtime() {
        let math = render_math_svg(r"\frac{1}{2} + x^2", false).unwrap();
        assert!(math.starts_with("<svg"));
        assert!(math.contains("<path"));

        let diagram = render_mermaid_svg("flowchart LR\nA[Start] --> B[Done]\n", false).unwrap();
        assert!(diagram.starts_with("<svg"));
        assert!(diagram.contains("Start"));

        let html =
            render_html_fragment("Inline $x^2$.\n\n```mermaid\nflowchart LR\nA --> B\n```\n");
        assert!(html.contains("math-inline"));
        assert!(html.contains("<svg"));
        assert!(!html.contains("<code class=\"language-mermaid\""));
    }

    #[test]
    fn rejects_pathologically_large_generated_content() {
        assert!(render_math_svg(&"x".repeat(MAX_MATH_BYTES + 1), true).is_err());
        assert!(render_mermaid_svg(&"x".repeat(MAX_MERMAID_BYTES + 1), false).is_err());
    }

    #[test]
    fn exported_html_removes_scripts_and_event_handlers() {
        let html = render_html_document(
            "<script>alert(1)</script><img src=\"safe.png\" onerror=\"alert(2)\">",
            "safe",
            false,
        );
        assert!(!html.contains("<script"));
        assert!(!html.contains("onerror"));
        assert!(html.contains("safe.png"));
    }

    #[test]
    fn generated_diagrams_do_not_reintroduce_active_html() {
        let html = render_html_document(
            "```mermaid\nflowchart LR\nA[<script>alert(1)</script>] --> B\n```\n",
            "safe diagram",
            false,
        );
        assert!(!html.to_ascii_lowercase().contains("<script"));
        assert!(!html.to_ascii_lowercase().contains("onload="));
    }

    #[test]
    fn generated_diagram_interactions_are_exported_as_static_svg() {
        let html = render_html_document(
            "```mermaid\nflowchart TD\nA[Open] --> B[Done]\nclick A runDanger \"callback\"\nclick B \"javascript:alert(1)\"\n```\n",
            "static diagram",
            false,
        );
        let lowercase = html.to_ascii_lowercase();
        assert!(lowercase.contains("<svg"));
        assert!(!lowercase.contains("onclick"));
        assert!(!lowercase.contains("javascript:"));
        assert!(!lowercase.contains("<script"));
    }

    #[test]
    fn html_fragments_are_sanitized_before_static_svg_is_inserted() {
        let html = render_html_fragment(
            "<img src=\"safe.png\" onerror=\"alert(1)\"><script>alert(2)</script>",
        );
        assert!(html.contains("safe.png"));
        assert!(!html.contains("onerror"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn rejects_oversized_generated_svg_output() {
        assert!(bound_generated_svg("x".repeat(MAX_GENERATED_SVG_BYTES + 1), "test").is_err());
    }

    #[test]
    fn rejects_generated_svg_with_excessive_geometry() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 128 1000000\"><path d=\"M0 0\"/></svg>";
        let error = bound_generated_svg(svg.to_owned(), "test").unwrap_err();
        assert!(error.contains("尺寸"));
    }

    #[test]
    fn rejects_generated_svg_with_degenerate_geometry() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"0\" height=\"0\"></svg>";
        assert!(bound_generated_svg(svg.to_owned(), "test").is_err());
    }

    #[test]
    fn generated_placeholder_text_cannot_expand_user_content() {
        let marker = "RUPORA_GENERATED_BLOCK_0_B5C4718D_0__";
        let html = render_html_fragment(&format!("{marker}\n\n$x$"));
        assert!(html.contains(marker));
        assert_eq!(html.matches("<svg").count(), 1);
    }

    #[test]
    fn caps_generated_blocks_per_document() {
        let source = std::iter::repeat_n("$x$", MAX_GENERATED_BLOCKS + 2)
            .collect::<Vec<_>>()
            .join(" ");
        let html = render_html_fragment(&source);
        assert!(html.matches("<svg").count() <= MAX_GENERATED_BLOCKS);
        assert!(html.contains("生成内容超过文档资源预算"));
    }

    #[test]
    fn collects_only_relative_document_links() {
        let links = local_link_destinations(
            "[local](notes/today.md) [anchor](#part) [web](https://example.com) \
             [mail](mailto:test@example.com) [local anchor](other.md#section)",
        );
        assert_eq!(links, vec!["notes/today.md", "other.md#section"]);
        assert!(!is_local_link(r"C:\notes\a.md"));
        assert!(!is_local_link(r"\\server\share\a.md"));
        assert!(!is_local_link("/absolute/a.md"));
    }

    #[test]
    fn native_hit_testing_finds_links_and_toggles_only_the_clicked_task() {
        let link = "before [文档](notes/today.md) after";
        let inside = link.find("文档").unwrap();
        assert_eq!(
            link_destination_at(link, inside).as_deref(),
            Some("notes/today.md")
        );
        assert!(link_destination_at(link, 0).is_none());

        let tasks = "- [ ] first\n- [x] second";
        let first = tasks.find("[ ]").unwrap() + 1;
        let updated = toggle_task_marker_at(tasks, first).unwrap();
        assert_eq!(updated, "- [x] first\n- [x] second");
        assert!(toggle_task_marker_at(tasks, tasks.find("first").unwrap()).is_none());
    }

    #[test]
    fn collects_relative_and_absolute_local_images_but_not_remote_ones() {
        let absolute = if cfg!(windows) {
            "C:/assets/absolute.png"
        } else {
            "/assets/absolute.png"
        };
        let images = local_image_destinations(&format!(
            "![local](assets/logo.png) ![absolute](<{absolute}>) \
             ![web](https://example.com/x.png) ![data](data:image/png;base64,AAAA) \
             ![again](assets/logo.png)"
        ));
        assert_eq!(
            images,
            vec![absolute.to_owned(), "assets/logo.png".to_owned()]
        );
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
    fn returns_an_editable_trailing_paragraph_after_a_fenced_block() {
        for source in ["```\ncode\n```\n\n", "```\r\ncode\r\n```\r\n\r\n"] {
            let blocks = blocks(source);
            assert_eq!(blocks.len(), 2, "source: {source:?}");
            assert_eq!(blocks[1].range, source.len()..source.len());
        }
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
    fn incomplete_heading_keeps_the_active_block_identity() {
        let mut index = BlockIndex::new("");
        let id = index.blocks()[0].id;
        for source in ["#", "# ", "# ATX 标题", "# ATX 标题\n"] {
            index.update(source);
            assert_eq!(index.blocks().len(), 1, "source: {source:?}");
            assert_eq!(index.blocks()[0].id, id, "source: {source:?}");
        }
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

    #[test]
    fn large_changed_gaps_preserve_ids_without_cartesian_candidates() {
        const BLOCK_COUNT: usize = 4_096;
        let original = (0..BLOCK_COUNT)
            .map(|index| format!("identity-{index:08}-before"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut index = BlockIndex::new(&original);
        let original_ids = index
            .blocks()
            .iter()
            .map(|block| block.id)
            .collect::<Vec<_>>();

        let mut changed_blocks = Vec::with_capacity(BLOCK_COUNT + 1);
        changed_blocks.push("entirely-new-leading-block".to_owned());
        changed_blocks.extend((0..BLOCK_COUNT).map(|block| format!("identity-{block:08}-after")));
        index.update(&changed_blocks.join("\n\n"));

        let changed_ids = index
            .blocks()
            .iter()
            .map(|block| block.id)
            .collect::<Vec<_>>();
        assert_eq!(changed_ids.len(), BLOCK_COUNT + 1);
        assert!(!original_ids.contains(&changed_ids[0]));
        assert_eq!(&changed_ids[1..], original_ids);
    }

    #[test]
    fn similarity_scoring_has_a_fixed_comparison_budget() {
        let common = "x".repeat(100_000);
        let left = format!("{common}left{common}");
        let right = format!("{common}right{common}");
        assert_eq!(
            similarity_score(&left, &right),
            MAX_SIMILARITY_CHARS_PER_SIDE * 2
        );
    }
}
