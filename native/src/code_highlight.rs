use std::sync::Arc;

use eframe::egui::{
    self, Color32, FontFamily, FontId, Ui,
    text::{LayoutJob, TextFormat},
};

const CODE_LINE_HEIGHT: f32 = 27.0;

#[derive(Clone, Copy)]
pub struct CodePalette {
    pub plain: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub comment: Color32,
    pub number: Color32,
}

pub fn layout(
    ui: &Ui,
    text: &str,
    wrap_width: f32,
    palette: CodePalette,
    language: Option<&str>,
) -> Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    job.keep_trailing_whitespace = true;
    for token in tokens(text, language) {
        let color = match token.kind {
            TokenKind::Plain => palette.plain,
            TokenKind::Keyword => palette.keyword,
            TokenKind::String => palette.string,
            TokenKind::Comment => palette.comment,
            TokenKind::Number => palette.number,
        };
        let mut format = TextFormat::simple(FontId::new(15.0, FontFamily::Monospace), color);
        format.line_height = Some(CODE_LINE_HEIGHT);
        job.append(&text[token.range], 0.0, format);
    }
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenKind {
    Plain,
    Keyword,
    String,
    Comment,
    Number,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    range: std::ops::Range<usize>,
    kind: TokenKind,
}

fn tokens(text: &str, language: Option<&str>) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut cursor = 0usize;
    let language = language.unwrap_or_default().to_ascii_lowercase();
    let hash_comments = matches!(
        language.as_str(),
        "py" | "python" | "rb" | "ruby" | "sh" | "bash" | "zsh" | "yaml" | "yml" | "toml"
    );

    while cursor < text.len() {
        let remaining = &text[cursor..];
        if !hash_comments && remaining.starts_with("//") {
            let end = remaining.find('\n').map_or(text.len(), |end| cursor + end);
            tokens.push(Token {
                range: cursor..end,
                kind: TokenKind::Comment,
            });
            cursor = end;
            continue;
        }
        if !hash_comments && let Some(comment_body) = remaining.strip_prefix("/*") {
            let end = comment_body
                .find("*/")
                .map_or(text.len(), |end| cursor + 2 + end + 2);
            tokens.push(Token {
                range: cursor..end,
                kind: TokenKind::Comment,
            });
            cursor = end;
            continue;
        }
        if hash_comments && remaining.starts_with('#') {
            let end = remaining.find('\n').map_or(text.len(), |end| cursor + end);
            tokens.push(Token {
                range: cursor..end,
                kind: TokenKind::Comment,
            });
            cursor = end;
            continue;
        }

        let character = remaining.chars().next().expect("cursor is in bounds");
        if matches!(character, '"' | '\'')
            && let Some(end) = quoted_token_end(text, cursor, character)
        {
            tokens.push(Token {
                range: cursor..end,
                kind: TokenKind::String,
            });
            cursor = end;
            continue;
        }
        if character.is_ascii_digit() {
            let end = cursor
                + remaining
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.'))
                    .count();
            tokens.push(Token {
                range: cursor..end,
                kind: TokenKind::Number,
            });
            cursor = end;
            continue;
        }
        if character == '_' || character.is_alphabetic() {
            let length = remaining
                .char_indices()
                .take_while(|(_, character)| *character == '_' || character.is_alphanumeric())
                .map(|(index, character)| index + character.len_utf8())
                .last()
                .unwrap_or(character.len_utf8());
            let end = cursor + length;
            let word = &text[cursor..end];
            tokens.push(Token {
                range: cursor..end,
                kind: if is_keyword(word, &language) {
                    TokenKind::Keyword
                } else {
                    TokenKind::Plain
                },
            });
            cursor = end;
            continue;
        }

        let end = cursor + character.len_utf8();
        tokens.push(Token {
            range: cursor..end,
            kind: TokenKind::Plain,
        });
        cursor = end;
    }
    tokens
}

fn quoted_token_end(text: &str, start: usize, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (offset, character) in text[start + quote.len_utf8()..].char_indices() {
        if character == '\n' && quote == '\'' {
            return None;
        }
        if character == quote && !escaped {
            return Some(start + quote.len_utf8() + offset + character.len_utf8());
        }
        escaped = character == '\\' && !escaped;
        if character != '\\' {
            escaped = false;
        }
    }
    None
}

fn is_keyword(word: &str, language: &str) -> bool {
    let common = matches!(
        word,
        "as" | "async"
            | "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "fn"
            | "for"
            | "from"
            | "function"
            | "if"
            | "impl"
            | "import"
            | "in"
            | "interface"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "new"
            | "nil"
            | "None"
            | "null"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "trait"
            | "true"
            | "try"
            | "type"
            | "typeof"
            | "unsafe"
            | "use"
            | "var"
            | "where"
            | "while"
            | "with"
            | "yield"
    );
    common
        || matches!(
            (language, word),
            (
                "python" | "py",
                "def" | "elif" | "except" | "lambda" | "pass" | "raise"
            ) | ("rust" | "rs", "crate" | "dyn" | "extern")
                | (
                    "go" | "golang",
                    "chan" | "defer" | "func" | "go" | "map" | "package" | "range" | "select"
                )
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_audit_python_floor_division_is_not_a_comment() {
        let source = "result = total // 2 # actual comment";
        let highlighted = tokens(source, Some("python"));
        let comments = highlighted
            .iter()
            .filter(|token| token.kind == TokenKind::Comment)
            .map(|token| &source[token.range.clone()])
            .collect::<Vec<_>>();
        assert_eq!(comments, vec!["# actual comment"]);
        assert!(highlighted.iter().any(|token| token.kind == TokenKind::Number && &source[token.range.clone()] == "2"));
    }

    #[test]
    fn preserves_unicode_and_classifies_core_tokens() {
        let source = "fn main() { let 名称 = \"值\"; // 注释\n  return 42; }";
        let tokens = tokens(source, Some("rust"));
        assert_eq!(
            tokens
                .iter()
                .map(|token| &source[token.range.clone()])
                .collect::<String>(),
            source
        );
        assert!(tokens.iter().any(|token| {
            token.kind == TokenKind::Keyword && &source[token.range.clone()] == "fn"
        }));
        assert!(tokens.iter().any(|token| {
            token.kind == TokenKind::String && &source[token.range.clone()] == "\"值\""
        }));
        assert!(tokens.iter().any(|token| {
            token.kind == TokenKind::Comment && source[token.range.clone()].starts_with("//")
        }));
        assert!(tokens.iter().any(|token| {
            token.kind == TokenKind::Number && &source[token.range.clone()] == "42"
        }));
    }
}
