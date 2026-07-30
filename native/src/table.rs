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
                Alignment::Left => format!(":{}", "-".repeat(width.saturating_sub(1))),
                Alignment::Center => format!(":{}:", "-".repeat(width.saturating_sub(2).max(1))),
                Alignment::Right => format!("{}:", "-".repeat(width.saturating_sub(1))),
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
    let lines = source_lines(source);
    let candidates = lines
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| parse_separator(pair[1].text).is_some())
        .filter_map(|(index, pair)| {
            let headers = parse_row(pair[0].text);
            let alignments = parse_separator(pair[1].text)?;
            (headers.len() == alignments.len() && !headers.is_empty())
                .then_some((index, headers, alignments))
        })
        .collect::<Vec<_>>();

    let mut fallback = None;
    for (header_index, headers, alignments) in candidates {
        let mut end_index = header_index + 2;
        let mut rows = Vec::new();
        while let Some(line) = lines.get(end_index) {
            if line.text.trim().is_empty() || !line.text.contains('|') {
                break;
            }
            let row = parse_row(line.text);
            if row.is_empty() {
                break;
            }
            rows.push(row);
            end_index += 1;
        }

        let range_start = lines[header_index].start;
        let range_end = lines[end_index.saturating_sub(1)].end;
        let mut table = MarkdownTable {
            range: range_start..range_end,
            headers,
            alignments,
            rows,
        };
        table.normalize();
        if table.range.contains(&cursor_byte) || cursor_byte == table.range.end {
            return Some(table);
        }
        fallback.get_or_insert(table);
    }
    fallback
}

pub fn new_table(insert_at: usize) -> MarkdownTable {
    MarkdownTable {
        range: insert_at..insert_at,
        headers: vec!["列 1".to_owned(), "列 2".to_owned()],
        alignments: vec![Alignment::None; 2],
        rows: vec![vec![String::new(); 2], vec![String::new(); 2]],
    }
}

#[derive(Clone, Copy)]
struct SourceLine<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

fn source_lines(source: &str) -> Vec<SourceLine<'_>> {
    let mut output = Vec::new();
    let mut start = 0usize;
    for line in source.split_inclusive('\n') {
        let end = start + line.len();
        output.push(SourceLine {
            start,
            end: end.saturating_sub(usize::from(line.ends_with('\n'))),
            text: line.trim_end_matches(['\r', '\n']),
        });
        start = end;
    }
    if source.is_empty() || source.ends_with('\n') {
        output.push(SourceLine {
            start,
            end: start,
            text: "",
        });
    }
    output
}

fn parse_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let content = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or_else(|| trimmed.strip_prefix('|').unwrap_or(trimmed));
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for character in content.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '|' {
            cells.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(character);
        }
    }
    if escaped {
        current.push('\\');
    }
    cells.push(current.trim().to_owned());
    cells
}

fn parse_separator(line: &str) -> Option<Vec<Alignment>> {
    let cells = parse_row(line);
    if cells.is_empty() {
        return None;
    }
    cells
        .into_iter()
        .map(|cell| {
            let trimmed = cell.trim();
            let left = trimmed.starts_with(':');
            let right = trimmed.ends_with(':');
            let dashes = trimmed.trim_matches(':');
            if dashes.len() < 3 || !dashes.bytes().all(|byte| byte == b'-') {
                return None;
            }
            Some(match (left, right) {
                (true, true) => Alignment::Center,
                (true, false) => Alignment::Left,
                (false, true) => Alignment::Right,
                (false, false) => Alignment::None,
            })
        })
        .collect()
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
}
