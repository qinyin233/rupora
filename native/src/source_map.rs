use std::ops::Range;

use pulldown_cmark::{Event, Parser, TagEnd};

use crate::markdown::parser_options;

/// Maps character boundaries in Markdown's rendered text back to UTF-8 source boundaries.
///
/// The map is exact when a parser event points at a literal source substring. For decoded
/// entities and other transformed events it preserves safe source boundaries and monotonicity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMap {
    rendered: String,
    source_boundaries: Vec<usize>,
}

impl SourceMap {
    pub fn from_markdown(source: &str) -> Self {
        let mut map = Self {
            rendered: String::new(),
            source_boundaries: vec![0],
        };

        for (event, range) in Parser::new_ext(source, parser_options()).into_offset_iter() {
            match event {
                Event::Text(text)
                | Event::Code(text)
                | Event::InlineMath(text)
                | Event::DisplayMath(text)
                | Event::Html(text)
                | Event::InlineHtml(text)
                | Event::FootnoteReference(text) => {
                    map.append_mapped(source, text.as_ref(), range);
                }
                Event::TaskListMarker(checked) => {
                    map.append_mapped(source, if checked { "[x] " } else { "[ ] " }, range);
                }
                Event::SoftBreak | Event::HardBreak => map.append_virtual("\n", range),
                Event::Rule => map.append_virtual("―\n", range),
                Event::End(
                    TagEnd::Paragraph
                    | TagEnd::Heading(_)
                    | TagEnd::CodeBlock
                    | TagEnd::Item
                    | TagEnd::TableRow,
                ) => map.ensure_line_break(range.end),
                _ => {}
            }
        }

        while map.rendered.ends_with('\n') {
            map.rendered.pop();
            map.source_boundaries.pop();
        }
        if map.source_boundaries.is_empty() {
            map.source_boundaries.push(0);
        }
        map
    }

    pub fn rendered_text(&self) -> &str {
        &self.rendered
    }

    pub fn rendered_char_count(&self) -> usize {
        self.source_boundaries.len().saturating_sub(1)
    }

    pub fn source_byte_for_rendered_char(&self, rendered_char: usize) -> usize {
        self.source_boundaries[rendered_char.min(self.rendered_char_count())]
    }

    pub fn rendered_char_for_source_byte(&self, source_byte: usize) -> usize {
        self.source_boundaries
            .partition_point(|boundary| *boundary < source_byte)
            .min(self.rendered_char_count())
    }

    pub fn source_byte_at_normalized_point(&self, x: f32, y: f32) -> usize {
        if self.rendered.is_empty() {
            return 0;
        }

        let mut lines = Vec::new();
        let mut start = 0usize;
        for (index, character) in self.rendered.chars().enumerate() {
            if character == '\n' {
                lines.push(start..index);
                start = index + 1;
            }
        }
        lines.push(start..self.rendered_char_count());

        let y = y.clamp(0.0, 0.999_999);
        let line_index = ((y * lines.len() as f32).floor() as usize).min(lines.len() - 1);
        let line = &lines[line_index];
        let x = x.clamp(0.0, 1.0);
        let column = (x * line.len() as f32).round() as usize;
        self.source_byte_for_rendered_char(line.start + column.min(line.len()))
    }

    fn append_mapped(&mut self, source: &str, rendered: &str, range: Range<usize>) {
        if rendered.is_empty() {
            return;
        }
        let source_fragment = &source[range.clone()];
        if let Some(relative_start) = source_fragment.find(rendered) {
            let source_start = range.start + relative_start;
            self.set_current_boundary(source_start);
            self.rendered.push_str(rendered);
            let mut consumed = 0usize;
            for character in rendered.chars() {
                consumed += character.len_utf8();
                self.source_boundaries.push(source_start + consumed);
            }
            return;
        }

        let source_chars = source_fragment
            .char_indices()
            .map(|(index, _)| range.start + index)
            .chain(std::iter::once(range.end))
            .collect::<Vec<_>>();
        let rendered_chars = rendered.chars().count();
        self.set_current_boundary(range.start);
        self.rendered.push_str(rendered);
        for index in 1..=rendered_chars {
            let source_index = index * source_chars.len().saturating_sub(1) / rendered_chars;
            self.source_boundaries.push(source_chars[source_index]);
        }
    }

    fn append_virtual(&mut self, rendered: &str, range: Range<usize>) {
        self.set_current_boundary(range.start);
        self.rendered.push_str(rendered);
        self.source_boundaries
            .extend(std::iter::repeat_n(range.end, rendered.chars().count()));
    }

    fn ensure_line_break(&mut self, source_byte: usize) {
        if !self.rendered.is_empty() && !self.rendered.ends_with('\n') {
            self.rendered.push('\n');
            self.source_boundaries.push(source_byte);
        }
    }

    fn set_current_boundary(&mut self, source_byte: usize) {
        if let Some(boundary) = self.source_boundaries.last_mut() {
            *boundary = source_byte;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_formatted_unicode_text_to_literal_source_boundaries() {
        let source = "**你🙂好**";
        let map = SourceMap::from_markdown(source);

        assert_eq!(map.rendered_text(), "你🙂好");
        assert_eq!(map.source_byte_for_rendered_char(0), 2);
        assert_eq!(map.source_byte_for_rendered_char(1), 5);
        assert_eq!(map.source_byte_for_rendered_char(2), 9);
        assert_eq!(map.source_byte_for_rendered_char(3), 12);
    }

    #[test]
    fn decoded_entities_remain_monotonic_and_on_utf8_boundaries() {
        let source = "甲 &amp; 乙";
        let map = SourceMap::from_markdown(source);

        assert_eq!(map.rendered_text(), "甲 & 乙");
        let boundaries = (0..=map.rendered_char_count())
            .map(|index| map.source_byte_for_rendered_char(index))
            .collect::<Vec<_>>();
        assert!(boundaries.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(
            boundaries
                .iter()
                .all(|boundary| source.is_char_boundary(*boundary))
        );
    }

    #[test]
    fn point_mapping_selects_the_corresponding_rendered_line() {
        let source = "first\n\nsecond";
        let map = SourceMap::from_markdown(source);

        let source_byte = map.source_byte_at_normalized_point(0.5, 0.75);
        assert!(source_byte >= source.find("second").unwrap());
        assert!(source.is_char_boundary(source_byte));
    }
}
