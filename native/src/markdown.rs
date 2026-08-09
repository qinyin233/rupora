use std::{
    collections::{HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
    sync::{Arc, OnceLock},
};

use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html,
};

const MAX_MATH_BYTES: usize = 16 * 1024;
const MAX_MERMAID_BYTES: usize = 256 * 1024;
const MAX_MERMAID_LINES: usize = 2_048;
const MAX_GENERATED_SVG_BYTES: usize = 8 * 1024 * 1024;
const MAX_GENERATED_SVG_EDGE: f32 = 8_192.0;
const MAX_GENERATED_SVG_PIXELS: f64 = 16.0 * 1024.0 * 1024.0;
const MAX_GENERATED_SVG_ASPECT_RATIO: f32 = 512.0;
const MAX_GENERATED_BLOCKS: usize = 128;
const MAX_GENERATED_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
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

pub fn heading_anchors(source: &str) -> Vec<HeadingAnchor> {
    let mut occurrences = HashMap::<String, usize>::new();
    analyze(source)
        .headings
        .into_iter()
        .map(|heading| {
            let base = heading_slug(&heading.text);
            let occurrence = occurrences.entry(base.clone()).or_default();
            let id = if *occurrence == 0 {
                base
            } else {
                format!("{base}-{}", *occurrence)
            };
            *occurrence += 1;
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

pub fn local_image_destinations(source: &str) -> Vec<String> {
    let mut destinations = Parser::new_ext(source, parser_options())
        .filter_map(|event| match event {
            Event::Start(Tag::Image { dest_url, .. }) => Some(dest_url.into_string()),
            _ => None,
        })
        .filter(|destination| is_local_link(destination))
        .collect::<Vec<_>>();
    destinations.sort();
    destinations.dedup();
    destinations
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

fn expand_front_matter_and_toc(source: &str) -> String {
    let (front_matter, body) = if let Some(front_matter) = parse_front_matter(source) {
        let body = &source[front_matter.body_start..];
        (Some(front_matter), body)
    } else {
        (None, source)
    };
    let anchors = heading_anchors(body);
    let toc = render_toc_markdown(&anchors);
    let mut output = String::new();

    if let Some(front_matter) = front_matter {
        output.push_str("> **文档元数据**\n");
        for (key, value) in front_matter.fields {
            output.push_str("> - **");
            output.push_str(&key.replace(['*', '[', ']'], ""));
            output.push_str("：** ");
            output.push_str(&value.replace('\n', " "));
            output.push('\n');
        }
        output.push('\n');
    }

    let mut in_fence = false;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if !in_fence && trimmed.eq_ignore_ascii_case("[TOC]") {
            output.push_str(&toc);
            if line.ends_with('\n') && !toc.ends_with('\n') {
                output.push('\n');
            }
        } else {
            output.push_str(line);
        }
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
    let mut output = String::new();
    for anchor in anchors {
        let indent = anchor.heading.level.saturating_sub(minimum_level) as usize;
        output.push_str(&"  ".repeat(indent));
        output.push_str("- [");
        output.push_str(
            &anchor
                .heading
                .text
                .replace('\\', "\\\\")
                .replace('[', "\\[")
                .replace(']', "\\]"),
        );
        output.push_str("](#");
        output.push_str(&anchor.id);
        output.push_str(")\n");
    }
    output
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
                id,
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
                        id: id.or_else(|| Some(CowStr::Boxed(generated_id.into_boxed_str()))),
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
    for salt in 0usize.. {
        let prefix = format!("RUPORA_GENERATED_BLOCK_{salt}_B5C4718D_");
        if !source.contains(&prefix) {
            return prefix;
        }
    }
    unreachable!("usize salt space cannot be exhausted")
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
    let aspect_ratio = width.max(height) / width.min(height);
    if !width.is_finite()
        || !height.is_finite()
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
    }

    #[test]
    fn collects_only_relative_document_images() {
        let images = local_image_destinations(
            "![local](assets/logo.png) ![web](https://example.com/x.png) \
             ![data](data:image/png;base64,AAAA) ![again](assets/logo.png)",
        );
        assert_eq!(images, vec!["assets/logo.png"]);
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
