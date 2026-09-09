use rupora::{
    document::{Document, EditKind},
    editing::{self, MarkdownCommand},
    table,
};

#[test]
fn audit_heading_anchors_are_unique_even_when_slugs_and_explicit_ids_collide() {
    let source = "[TOC]\n\n# Foo\n# Foo\n# Foo-1\n# Explicit {#foo}\n# Again {#foo}\n";
    let anchors = rupora::markdown::heading_anchors(source);
    let unique = anchors
        .iter()
        .map(|anchor| &anchor.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), anchors.len());
    let html = rupora::markdown::render_html_document(source, "anchors", false);
    for anchor in anchors {
        assert_eq!(html.matches(&format!("id=\"{}\"", anchor.id)).count(), 1);
        assert!(html.contains(&format!("href=\"#{}\"", anchor.id)));
    }
}

#[test]
fn audit_bulk_replace_preserves_unicode_and_does_not_reprocess_replacements() {
    let mut source = "你 Aa aA 🙂 Aa 终".to_owned();
    assert_eq!(editing::replace_all(&mut source, "aa", "AaAa🙂", false), 3);
    assert_eq!(source, "你 AaAa🙂 AaAa🙂 🙂 AaAa🙂 终");
    assert_eq!(
        editing::find_previous(&source, "AaAa🙂", 100, true),
        Some(16..21)
    );
    assert_eq!(editing::replace_all(&mut source, "AaAa🙂", "", true), 3);
    assert_eq!(source, "你   🙂  终");
}

#[test]
#[ignore = "manual bulk replacement timing; enforced in the release perf_guard"]
fn audit_measure_bulk_replace() {
    let mut source = "中文 Aa 🙂\n".repeat(25_000);
    let started = std::time::Instant::now();
    assert_eq!(
        editing::replace_all(&mut source, "aa", "替换", false),
        25_000
    );
    eprintln!("replace 25k matches: {:?}", started.elapsed());
}

#[test]
fn audit_line_commands_do_not_touch_the_line_at_the_exclusive_selection_end() {
    for command in [
        MarkdownCommand::Quote,
        MarkdownCommand::BulletList,
        MarkdownCommand::Heading(2),
    ] {
        let mut text = "中文🙂\n下一行".to_owned();
        editing::apply_markdown_command(&mut text, 0..4, command);
        assert!(text.ends_with("\n下一行"), "{command:?}: {text:?}");
    }
    let mut text = "中文🙂\n下一行".to_owned();
    editing::indent_selected_lines(&mut text, 0..4, false);
    assert_eq!(text, "    中文🙂\n下一行");
    let mut text = "    中文🙂\n    下一行".to_owned();
    editing::indent_selected_lines(&mut text, 0..8, true);
    assert_eq!(text, "中文🙂\n    下一行");
}

#[test]
fn audit_table_editor_does_not_select_an_unrelated_table() {
    let source = "| a |\n| --- |\n| b |\n\n在这里插入🙂";
    assert!(table::find_table(source, source.len()).is_none());
}

#[test]
fn audit_table_editor_ignores_code_and_setext_headings() {
    for source in [
        "```md\n| a |\n| --- |\n| b |\n```",
        "标题\n---",
        "    | a |\n    | --- |\n    | b |",
    ] {
        assert!(
            table::find_table(source, source.find("---").unwrap()).is_none(),
            "{source:?}"
        );
    }
}

#[test]
fn audit_table_rows_stop_before_other_markdown_blocks() {
    let source = "| a |\n| --- |\n| b |\n# 标题 | 正文\n";
    let table = table::find_table(source, 0).unwrap();
    assert_eq!(table.rows, vec![vec!["b"]]);
    assert!(!source[table.range].contains("标题"));
}

#[test]
fn audit_table_keeps_an_escaped_pipe_at_the_end_of_an_unbordered_row() {
    let source = "a | b\n--- | ---\nx | end\\|";
    let table = table::find_table(source, 0).unwrap();
    assert_eq!(table.rows[0], vec!["x", "end|"]);
}

#[test]
fn audit_successful_save_ends_the_current_typing_undo_group() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("typing.md");
    std::fs::write(&path, "").unwrap();
    let mut document = Document::open(&path).unwrap();
    document.content = "你".to_owned();
    document.record_edit(String::new(), Some(0..0), Some(1..1), EditKind::Typing);
    document.save(false).unwrap();
    document.content.push('🙂');
    document.record_edit("你".to_owned(), Some(1..1), Some(2..2), EditKind::Typing);
    document.undo().unwrap();
    assert_eq!(document.content, "你");
    assert!(!document.dirty);
}
