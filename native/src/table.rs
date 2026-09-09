use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownTable {
    pub range: Range<usize>,
    pub headers: Vec<String>,
    pub alignments: Vec<Alignment>,
    pub rows: Vec<Vec<String>>,
}

impl MarkdownTable {
    pub fn normalize(&mut self) {
        let columns = self.headers.len().max(1);
        self.headers.resize(columns, String::new());
        self.alignments.resize(columns, Alignment::None);
        self.alignments.truncate(columns);
        for row in &mut self.rows {
            row.resize(columns, String::new());
            row.truncate(columns);
        }
    }

    pub fn add_column(&mut self) {
        self.headers.push("列".to_owned());
        self.alignments.push(Alignment::None);
        for row in &mut self.rows {
            row.push(String::new());
        }
    }

    pub fn remove_column(&mut self) {
        if self.headers.len() <= 1 {
            return;
        }
        self.headers.pop();
        self.alignments.pop();
        for row in &mut self.rows {
            row.pop();
        }
    }

    pub fn add_row(&mut self) {
        self.rows.push(vec![String::new(); self.headers.len()]);
    }

    pub fn remove_row(&mut self) {
        self.rows.pop();
    }

    pub fn to_markdown(&self) -> String {
        let mut table = self.clone();
        table.normalize();
        let mut widths = table
            .headers
            .iter()
            .map(|cell| escaped_cell(cell).chars().count().max(3))
            .collect::<Vec<_>>();
        for row in &table.rows {
            for (index, cell) in row.iter().enumerate() {
                widths[index] = widths[index].max(escaped_cell(cell).chars().count());
            }
        }

        let mut output = String::new();
        push_row(&mut output, &table.headers, &widths);
        output.push('|');
        for (index, alignment) in table.alignments.iter().enumerate() {
            let width = widths[index].max(3);
            let separator = match alignment {
                Alignment::None => "-".repeat(width),
                Alignment::Left => format!(":{}", "-".repeat(width)),
                Alignment::Center => format!(":{}:", "-".repeat(width)),
                Alignment::Right => format!("{}:", "-".repeat(width)),
            };
            output.push(' ');
            output.push_str(&separator);
            output.push(' ');
            output.push('|');
        }
        output.push('\n');
        for row in &table.rows {
            push_row(&mut output, row, &widths);
        }
        output.trim_end_matches('\n').to_owned()
    }
}

pub fn find_table(source: &str, cursor_byte: usize) -> Option<MarkdownTable> {
    // The Markdown parser owns block boundaries. A line scanner also mistakes
    // fenced examples and Setext headings for tables, and swallows later blocks.
    let body_start =
        crate::markdown::parse_front_matter(source).map_or(0, |front| front.body_start);
    let mut depth = 0usize;
    for (event, range) in
        pulldown_cmark::Parser::new_ext(&source[body_start..], crate::markdown::parser_options())
            .into_offset_iter()
    {
        match event {
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Table(alignments)) if depth == 0 => {
                let start = body_start + range.start;
                let text = source[start..body_start + range.end].trim_end_matches(['\r', '\n']);
                let range = start..start + text.len();
                if range.contains(&cursor_byte) || cursor_byte == range.end {
                    let mut lines = text.lines();
                    let headers = parse_row(lines.next()?);
                    lines.next()?; // delimiter row
                    let mut table = MarkdownTable {
                        range,
                        headers,
                        alignments: alignments
                            .into_iter()
                            .map(|alignment| match alignment {
                                pulldown_cmark::Alignment::None => Alignment::None,
                                pulldown_cmark::Alignment::Left => Alignment::Left,
                                pulldown_cmark::Alignment::Center => Alignment::Center,
                                pulldown_cmark::Alignment::Right => Alignment::Right,
                            })
                            .collect(),
                        rows: lines.map(parse_row).collect(),
                    };
                    table.normalize();
                    return Some(table);
                }
                depth += 1;
            }
            pulldown_cmark::Event::Start(_) => depth += 1,
            pulldown_cmark::Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

pub fn new_table(insert_at: usize) -> MarkdownTable {
    MarkdownTable {
        range: insert_at..insert_at,
        headers: vec!["列 1".to_owned(), "列 2".to_owned()],
        alignments: vec![Alignment::None; 2],
        rows: vec![vec![String::new(); 2], vec![String::new(); 2]],
    }
}

fn parse_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let content = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let content = if let Some(prefix) = content.strip_suffix('|')
        && prefix
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'\\')
            .count()
            .is_multiple_of(2)
    {
        prefix
    } else {
        content
    };
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut characters = content.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' && matches!(characters.peek(), Some('|' | '\\')) {
            current.push(characters.next().expect("peeked escaped table character"));
        } else if character == '|' {
            cells.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(character);
        }
    }
    cells.push(current.trim().to_owned());
    cells
}

fn escaped_cell(cell: &str) -> String {
    cell.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\r', '\n'], " ")
}

fn push_row(output: &mut String, cells: &[String], widths: &[usize]) {
    output.push('|');
    for (index, cell) in cells.iter().enumerate() {
        let escaped = escaped_cell(cell);
        output.push(' ');
        output.push_str(&escaped);
        output.push_str(&" ".repeat(widths[index].saturating_sub(escaped.chars().count())));
        output.push(' ');
        output.push('|');
    }
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_serializes_alignment_unicode_and_escaped_pipes() {
        let source = "before\n\n| 名称 | Value |\n| :--- | ---: |\n| 甲\\|乙 | 42 |\n\nafter";
        let cursor = source.find("42").unwrap();
        let table = find_table(source, cursor).unwrap();

        assert_eq!(table.headers, vec!["名称", "Value"]);
        assert_eq!(table.alignments, vec![Alignment::Left, Alignment::Right]);
        assert_eq!(table.rows[0], vec!["甲|乙", "42"]);
        assert!(table.to_markdown().contains("甲\\|乙"));
    }

    #[test]
    fn preserves_literal_backslashes_and_accepts_compact_gfm_separators() {
        let source = "| Path | Literal |\n| - | :-: |\n| C:\\Temp | a\\\\b |";
        let table = find_table(source, source.find("Temp").unwrap()).unwrap();

        assert_eq!(table.rows[0], vec![r"C:\Temp", r"a\b"]);
        let serialized = table.to_markdown();
        assert!(serialized.contains(r"C:\\Temp"));
        assert!(serialized.contains(r"a\\b"));
        assert_eq!(
            find_table(&serialized, 0).unwrap().rows,
            table.rows,
            "opening and applying the table editor must be lossless"
        );
    }

    #[test]
    fn chooses_the_table_containing_the_cursor() {
        let source = "| a |\n| --- |\n| 1 |\n\ntext\n\n| b |\n| --- |\n| second |\n";
        let table = find_table(source, source.find("second").unwrap()).unwrap();
        assert_eq!(table.headers, vec!["b"]);
    }

    #[test]
    fn keeps_every_row_rectangular_when_columns_change() {
        let mut table = new_table(0);
        table.add_column();
        assert!(table.rows.iter().all(|row| row.len() == 3));
        table.remove_column();
        table.remove_column();
        table.remove_column();
        assert_eq!(table.headers.len(), 1);
        assert!(table.rows.iter().all(|row| row.len() == 1));
    }

    #[test]
    fn serializes_empty_centered_headers_as_a_parseable_table() {
        let table = MarkdownTable {
            range: 0..0,
            headers: vec![String::new(); 3],
            alignments: vec![Alignment::Center; 3],
            rows: Vec::new(),
        };
        let markdown = table.to_markdown();
        let reparsed = find_table(&markdown, 0).unwrap();
        assert_eq!(reparsed.headers.len(), 3);
        assert_eq!(reparsed.alignments, vec![Alignment::Center; 3]);
    }
}
