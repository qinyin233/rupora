//! Standard inline HTML emphasis used when Markdown delimiter flanking cannot
//! express a selection. Only balanced, attribute-free tags are promoted; the
//! Markdown parser remains responsible for code, escapes and block boundaries.
use std::collections::BTreeSet;
use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InlineFormat {
    Strong,
    Emphasis,
}

impl InlineFormat {
    pub(crate) fn for_marker(marker: &str) -> Option<Self> {
        match marker {
            "**" => Some(Self::Strong),
            "*" => Some(Self::Emphasis),
            _ => None,
        }
    }

    pub(crate) fn html(self) -> (&'static str, &'static str) {
        match self {
            Self::Strong => ("<strong>", "</strong>"),
            Self::Emphasis => ("<em>", "</em>"),
        }
    }

    fn tag(self) -> Tag<'static> {
        match self {
            Self::Strong => Tag::Strong,
            Self::Emphasis => Tag::Emphasis,
        }
    }
}

pub(crate) struct FormatSpan {
    pub(crate) kind: InlineFormat,
    pub(crate) syntax: Range<usize>,
    pub(crate) body: Range<usize>,
    pub(crate) html: bool,
}

/// Outermost-first wrappers whose body is exactly the selection, possibly
/// through other complete wrappers. Index each range once instead of walking
/// every descendant again for each ancestor in a deeply nested document.
pub(crate) fn matching_body<'a>(
    spans: &'a [FormatSpan],
    selected: &Range<usize>,
) -> Vec<&'a FormatSpan> {
    let mut bodies = BTreeSet::from([(selected.start, selected.end)]);
    let mut matching = Vec::new();
    for span in spans.iter().rev() {
        if bodies.contains(&(span.body.start, span.body.end)) {
            bodies.insert((span.syntax.start, span.syntax.end));
            matching.push(span);
        }
    }
    matching.reverse();
    matching
}

pub(crate) fn spans(source: &str) -> Vec<FormatSpan> {
    events(
        source,
        Parser::new_ext(source, crate::markdown::parser_options()).into_offset_iter(),
    )
    .filter_map(|(event, syntax)| {
        let kind = match event {
            Event::Start(Tag::Strong) => InlineFormat::Strong,
            Event::Start(Tag::Emphasis) => InlineFormat::Emphasis,
            _ => return None,
        };
        let (open, close) = kind.html();
        let html = source[syntax.clone()].starts_with(open);
        let body = if html {
            syntax.start + open.len()..syntax.end - close.len()
        } else {
            let width = if kind == InlineFormat::Strong { 2 } else { 1 };
            syntax.start + width..syntax.end - width
        };
        Some(FormatSpan {
            kind,
            syntax,
            body,
            html,
        })
    })
    .collect()
}

pub(crate) fn events<'a>(
    source: &str,
    input: impl Iterator<Item = (Event<'a>, Range<usize>)>,
) -> impl Iterator<Item = (Event<'a>, Range<usize>)> {
    // Keep the common Markdown-only path streaming and allocation-free.
    let mut raw = Some(input);
    let promoted = ["<strong>", "<em>"]
        .iter()
        .any(|tag| source.contains(tag))
        .then(|| promote(raw.take().unwrap().collect()));
    promoted
        .into_iter()
        .flatten()
        .chain(raw.into_iter().flatten())
}

fn promote(mut input: Vec<(Event<'_>, Range<usize>)>) -> Vec<(Event<'_>, Range<usize>)> {
    let mut depth = 0usize;
    let mut pending = Vec::<(InlineFormat, usize, usize)>::new();
    let mut pairs = Vec::new();
    for (index, (event, _)) in input.iter().enumerate() {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                while pending
                    .last()
                    .is_some_and(|(_, _, opened_depth)| *opened_depth > depth)
                {
                    pending.pop();
                }
            }
            Event::InlineHtml(html) => {
                // The editor uses neutral comments/spans to retain Markdown
                // whitespace. They do not open a competing HTML style scope.
                if (html.starts_with("<!--") && html.ends_with("-->"))
                    || matches!(html.as_ref(), "<span>" | "</span>")
                {
                    continue;
                }
                let tag = match html.as_ref() {
                    "<strong>" => Some((InlineFormat::Strong, true)),
                    "</strong>" => Some((InlineFormat::Strong, false)),
                    "<em>" => Some((InlineFormat::Emphasis, true)),
                    "</em>" => Some((InlineFormat::Emphasis, false)),
                    _ => None,
                };
                match tag {
                    Some((kind, true)) => pending.push((kind, index, depth)),
                    Some((kind, false)) => {
                        if let Some((opened, start, opened_depth)) = pending.pop()
                            && opened == kind
                            && opened_depth == depth
                        {
                            pairs.push((kind, start, index));
                        } else {
                            pending.clear();
                        }
                    }
                    None => pending.clear(),
                }
            }
            Event::Html(_) => pending.clear(),
            _ => {}
        }
    }
    for (kind, start, end) in pairs {
        let syntax = input[start].1.start..input[end].1.end;
        input[start] = (Event::Start(kind.tag()), syntax.clone());
        let end_tag = match kind {
            InlineFormat::Strong => TagEnd::Strong,
            InlineFormat::Emphasis => TagEnd::Emphasis,
        };
        input[end] = (Event::End(end_tag), syntax);
    }
    input
}
