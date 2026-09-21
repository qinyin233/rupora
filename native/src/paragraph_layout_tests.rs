use std::path::{Path, PathBuf};

use eframe::egui::{self, Context, FontDefinitions, FontFamily, Key, TextEdit, text::CCursor};

use crate::{
    document::Document,
    presentation::{WYSIWYG_BODY_LINE_HEIGHT, WYSIWYG_STRONG_FAMILY},
    wysiwyg::VisualProjection,
};

use super::{EditorBookmark, EditorOptions, EditorSurface, ViewMode};

struct Harness {
    document: Document,
    surface: EditorSurface,
    base_path: PathBuf,
}

fn context() -> Context {
    let ctx = Context::default();
    let mut fonts = FontDefinitions::default();
    fonts.families.insert(
        FontFamily::Name(WYSIWYG_STRONG_FAMILY.into()),
        fonts.families[&FontFamily::Proportional].clone(),
    );
    ctx.set_fonts(fonts);
    ctx
}

fn app(source: &str, directory: &Path) -> Harness {
    let mut document = Document::untitled(1);
    document.content = source.into();
    document.update_after_edit();
    let mut surface = EditorSurface::default();
    surface.bind_document(Some(&document), EditorBookmark::default());
    Harness {
        document,
        surface,
        base_path: directory.to_path_buf(),
    }
}

fn frame(app: &mut Harness, ctx: &Context, events: Vec<egui::Event>) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            let output = app.surface.show(
                ui,
                &mut app.document,
                EditorOptions {
                    mode: ViewMode::Hybrid,
                    dark: false,
                    base_path: &app.base_path,
                },
            );
            assert!(
                output.destination.is_none(),
                "paragraph editing must not request navigation"
            );
            assert!(
                output.commands.is_empty(),
                "paragraph editing must complete without shell commands"
            );
        },
    )
}

fn position(output: &egui::FullOutput, marker: &str) -> egui::Pos2 {
    fn find(shape: &egui::Shape, marker: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(text) => {
                let byte = text.galley.job.text.find(marker)?;
                let index = text.galley.job.text[..byte].chars().count();
                Some(text.galley.pos_from_cursor(CCursor::new(index)).min + text.pos.to_vec2())
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, marker)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, marker))
        .unwrap_or_else(|| panic!("missing {marker}"))
}

fn click(app: &mut Harness, ctx: &Context, at: egui::Pos2) -> egui::FullOutput {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    frame(app, ctx, vec![])
}

#[test]
fn paragraph_click_keeps_text_and_following_block_positions() {
    let mut failures = Vec::new();
    for first in [
        "FIRST paragraph.".to_owned(),
        "FIRST line\nsoft continuation.".into(),
        format!("FIRST {}", "wrapped text ".repeat(25)),
        "# FIRST heading".into(),
        "> FIRST quote".into(),
        "- FIRST item\n- Second item".into(),
        "    FIRST code\n    continuation".into(),
        "```\nFIRST code\ncontinuation\n```".into(),
        "~~~rust\nFIRST code\ncontinuation\n~~~".into(),
        "FIRST line\r\nsoft continuation.".into(),
        "| FIRST | Column |\n| --- | --- |\n| Cell | Cell |".into(),
        "- FIRST item\n  - Nested item\n- Final item".into(),
        "> FIRST line\n>\n> Next paragraph".into(),
    ] {
        for breaks in [2, 3, 4] {
            let directory = tempfile::tempdir().unwrap();
            let source = format!("{first}{}NEXT_PARAGRAPH\n\nTAIL", "\n".repeat(breaks));
            let mut app = app(&source, directory.path());
            let ctx = context();
            frame(&mut app, &ctx, vec![]);
            let before = frame(&mut app, &ctx, vec![]);
            let text = position(&before, "FIRST");
            let next = position(&before, "NEXT_PARAGRAPH");
            let after = click(&mut app, &ctx, text + egui::vec2(12.0, 8.0));
            let text_after = position(&after, "FIRST");
            let next_after = position(&after, "NEXT_PARAGRAPH");
            if text.distance(text_after) > 1.0 || next.distance(next_after) > 1.0 {
                failures.push(format!("first={first:?} breaks={breaks}: text {text:?}->{text_after:?}; next {next:?}->{next_after:?}"));
            }
            assert_eq!(app.document.content, source);
            let switched = click(&mut app, &ctx, next_after + egui::vec2(12.0, 8.0));
            assert!(position(&switched, "FIRST").distance(text) <= 1.0);
            assert!(position(&switched, "NEXT_PARAGRAPH").distance(next) <= 1.0);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn key(key: Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn enter_at_paragraph_end_keeps_the_new_line_before_the_next_paragraph() {
    let directory = tempfile::tempdir().unwrap();
    let source = "FIRST\n\nNEXT_PARAGRAPH";
    let mut app = app(source, directory.path());
    app.surface.queue_editor_selection(5..5);
    let ctx = context();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(Key::Enter)]);
    frame(&mut app, &ctx, vec![egui::Event::Text("NEW".into())]);
    assert_eq!(app.document.content, "FIRST\n\nNEW\n\nNEXT_PARAGRAPH");
}

#[test]
fn deleting_a_visible_blank_row_preserves_the_paragraph_separator() {
    for breaks in [3, 4] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("FIRST{}NEXT_PARAGRAPH", "\n".repeat(breaks));
        let mut app = app(&source, directory.path());
        let cursor = 5 + breaks - 1;
        app.surface.queue_editor_selection(cursor..cursor);
        let ctx = context();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![key(Key::Backspace)]);
        assert_eq!(app.document.content, "FIRST\n\nNEXT_PARAGRAPH");
    }
}

#[test]
fn clicking_real_blank_space_keeps_layout_and_inserts_before_next_paragraph() {
    let directory = tempfile::tempdir().unwrap();
    let source = "FIRST\n\n\nNEXT_PARAGRAPH\n\nTAIL";
    let mut app = app(source, directory.path());
    let ctx = context();
    frame(&mut app, &ctx, vec![]);
    let before = frame(&mut app, &ctx, vec![]);
    let next = position(&before, "NEXT_PARAGRAPH");
    let first = position(&before, "FIRST");
    let at = egui::pos2(next.x + 12.0, (first.y + next.y) * 0.5 + 8.0);
    let after = click(&mut app, &ctx, at);
    assert!(
        position(&after, "NEXT_PARAGRAPH").distance(next) <= 1.0,
        "blank click: next {next:?} -> {:?}",
        position(&after, "NEXT_PARAGRAPH")
    );
    assert_eq!(app.document.content, source);
    frame(&mut app, &ctx, vec![egui::Event::Text("NEW🙂".into())]);
    assert_eq!(
        app.document.content,
        "FIRST\n\nNEW🙂\n\nNEXT_PARAGRAPH\n\nTAIL"
    );
}

#[test]
fn final_paragraph_keeps_existing_trailing_rows_when_clicked() {
    fn rect(shape: &egui::Shape) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text.contains("FIRST") => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(rect),
            _ => None,
        }
    }
    for breaks in 0..4 {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("PREFIX\n\nFIRST{}", "\n".repeat(breaks));
        let mut app = app(&source, directory.path());
        let ctx = context();
        frame(&mut app, &ctx, vec![]);
        let before = frame(&mut app, &ctx, vec![]);
        let at = position(&before, "FIRST") + egui::vec2(12.0, 8.0);
        let before_rect = before
            .shapes
            .iter()
            .find_map(|shape| rect(&shape.shape))
            .unwrap();
        let after = click(&mut app, &ctx, at);
        let after_rect = after
            .shapes
            .iter()
            .find_map(|shape| rect(&shape.shape))
            .unwrap();
        assert_eq!(before_rect, after_rect, "trailing line breaks = {breaks}");
        assert_eq!(app.document.content, source);
    }
}

#[test]
fn arrows_cross_hidden_paragraph_separators_without_editing() {
    for (source, cursor, action, expected) in [
        ("FIRST\n\nNEXT", 5, Key::ArrowRight, 7),
        ("FIRST\n\nNEXT", 5, Key::ArrowDown, 7),
        ("FIRST\n\nNEXT", 7, Key::ArrowLeft, 5),
        ("FIRST\n\nNEXT", 7, Key::ArrowUp, 5),
        ("FIRST\n\n\nNEXT", 7, Key::ArrowRight, 8),
        ("FIRST\n\n```\ncode\n```", 5, Key::ArrowRight, 11),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app(source, directory.path());
        app.surface.queue_editor_selection(cursor..cursor);
        let ctx = context();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![key(action)]);
        assert_eq!(app.document.content, source);
        assert_eq!(
            app.surface.cursor().unwrap().primary.index.0,
            expected,
            "{action:?} {source:?}"
        );
        frame(&mut app, &ctx, vec![egui::Event::Text("X".into())]);
        let mut expected_text = source.to_owned();
        expected_text.insert(expected, 'X');
        assert_eq!(app.document.content, expected_text);
    }
}

#[test]
fn typing_in_the_last_visible_blank_row_keeps_next_paragraph_separate() {
    let directory = tempfile::tempdir().unwrap();
    let source = "FIRST\n\n\nNEXT_PARAGRAPH";
    let mut app = app(source, directory.path());
    app.surface
        .queue_editor_selection("FIRST\n\n".chars().count().."FIRST\n\n".chars().count());
    let ctx = context();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![egui::Event::Text("NEW".into())]);
    assert_eq!(app.document.content, "FIRST\n\nNEW\n\nNEXT_PARAGRAPH");
    app.surface.apply_history(&mut app.document, false);
    assert_eq!(app.document.content, source);
}

#[test]
fn newline_audit_enter_at_document_start_creates_visible_space_before_text() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app("FIRST\n\nNEXT", directory.path());
    app.surface.queue_editor_selection(0..0);
    let ctx = context();
    let before = frame(&mut app, &ctx, vec![]);
    let first = position(&before, "FIRST");
    frame(&mut app, &ctx, vec![key(Key::Enter)]);
    let after = frame(&mut app, &ctx, vec![]);
    assert!(
        position(&after, "FIRST").y > first.y + 10.0,
        "Enter at the beginning must create a visible line before FIRST: {:?}",
        app.document.content
    );
}

#[test]
fn newline_audit_typing_after_enter_stays_on_the_visible_blank_line() {
    fn caret(app: &Harness, ctx: &Context, output: &egui::FullOutput) -> egui::Pos2 {
        let id = app.surface.widget_id().unwrap();
        let state = TextEdit::load_state(ctx, id).unwrap();
        let cursor = state.cursor.char_range().unwrap().primary;
        let origin = ctx.read_response(id).unwrap().rect.min;
        fn find(shape: &egui::Shape, cursor: CCursor, origin: egui::Pos2) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.pos.distance(origin) < 1.0 => {
                    Some(text.galley.pos_from_cursor(cursor).min + text.pos.to_vec2())
                }
                egui::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| find(shape, cursor, origin))
                }
                _ => None,
            }
        }
        // Empty paragraphs have their own editor, so locate the focused widget
        // instead of finding FIRST in a different, read-only block.
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, cursor, origin))
            .unwrap_or(origin)
    }
    let mut failures = Vec::new();
    for source in ["FIRST", "FIRST\n\nNEXT"] {
        for count in 1..=3 {
            let directory = tempfile::tempdir().unwrap();
            let mut app = app(source, directory.path());
            app.surface.queue_editor_selection(5..5);
            let ctx = context();
            frame(&mut app, &ctx, vec![]);
            for _ in 0..count {
                frame(&mut app, &ctx, vec![key(Key::Enter)]);
            }
            let empty = frame(&mut app, &ctx, vec![]);
            let before = caret(&app, &ctx, &empty);
            frame(&mut app, &ctx, vec![egui::Event::Text("NEW".into())]);
            let typed = frame(&mut app, &ctx, vec![]);
            let after = caret(&app, &ctx, &typed);
            if (after.y - before.y).abs() > 1.0 {
                failures.push(format!(
                    "source={source:?}, Enter x{count}, caret y {} -> {}, text {:?}",
                    before.y, after.y, app.document.content
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn newline_audit_repeated_shift_enter_keeps_caret_and_following_paragraph_stable() {
    for count in 1..=3 {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app("FIRST\n\nNEXT", directory.path());
        app.surface.queue_editor_selection(5..5);
        let ctx = context();
        frame(&mut app, &ctx, vec![]);
        for _ in 0..count {
            let mut enter = key(Key::Enter);
            if let egui::Event::Key { modifiers, .. } = &mut enter {
                *modifiers = egui::Modifiers::SHIFT;
            }
            frame(&mut app, &ctx, vec![enter]);
        }
        let before = frame(&mut app, &ctx, vec![]);
        let next = position(&before, "NEXT");
        let body = app.document.blocks()[0].range.clone();
        assert_eq!(
            VisualProjection::from_markdown(&app.document.content[body]).text(),
            format!("FIRST{}", "\n".repeat(count)),
            "empty hard-break rows must not expose Markdown syntax"
        );
        let typed = frame(&mut app, &ctx, vec![egui::Event::Text("NEW".into())]);
        let after = frame(&mut app, &ctx, vec![]);
        assert!(
            (position(&after, "NEXT").y - next.y).abs() <= 1.0,
            "Shift+Enter x{count} changed following layout on typing: {:?} next {:?}",
            app.document.content,
            position(&after, "NEXT")
        );
        assert!(
            (position(&typed, "NEW").y
                - position(&typed, "FIRST").y
                - count as f32 * WYSIWYG_BODY_LINE_HEIGHT)
                .abs()
                <= 1.0
        );
        assert_eq!(
            app.document.content,
            format!("FIRST  \n{}NEW\n\nNEXT", "\\\n".repeat(count - 1))
        );
    }
}
