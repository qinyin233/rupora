use std::{ops::Range, path::Path};

use eframe::egui::{
    self, Context, FontDefinitions, FontFamily,
    text::{CCursor, CCursorRange},
};

use crate::{
    document::{Document, EditKind},
    presentation::WYSIWYG_STRONG_FAMILY,
};

use super::{EditorBookmark, EditorOptions, EditorOutput, EditorSurface, ViewMode};

const WIDTH: f32 = 1200.0;

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

fn document(source: &str) -> Document {
    // Deliberately reuse the untitled label: session state belongs to Document::id().
    let mut document = Document::untitled(1);
    document.content = source.to_owned();
    document.update_after_edit();
    document
}

fn bookmark(selection: Range<usize>) -> EditorBookmark {
    EditorBookmark {
        cursor: Some(CCursorRange::two(
            CCursor::new(selection.start),
            CCursor::new(selection.end),
        )),
        scroll_ratio: 0.0,
    }
}

fn frame(
    surface: &mut EditorSurface,
    document: &mut Document,
    ctx: &Context,
    mode: ViewMode,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, EditorOutput) {
    let mut effects = EditorOutput::default();
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WIDTH, 800.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            effects = surface.show(
                ui,
                document,
                EditorOptions {
                    mode,
                    dark: false,
                    base_path: Path::new("."),
                },
            );
        },
    );
    assert!(effects.destination.is_none());
    assert!(
        effects.commands.is_empty(),
        "editing and deferred history must complete inside the surface"
    );
    (output, effects)
}

fn painted_text(output: &egui::FullOutput, minimum_x: f32) -> String {
    fn collect(shape: &egui::Shape, minimum_x: f32, text: &mut String) {
        match shape {
            egui::Shape::Text(shape) if shape.pos.x >= minimum_x => {
                text.push_str(&shape.galley.job.text);
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, minimum_x, text);
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    for shape in &output.shapes {
        collect(&shape.shape, minimum_x, &mut text);
    }
    text
}

fn assert_split_preview(output: &egui::FullOutput, expected: &str) {
    assert_eq!(
        painted_text(output, WIDTH / 2.0),
        expected,
        "the right pane must paint the document after this frame's history operation"
    );
}

#[test]
fn full_audit_source_shift_tab_uses_the_key_event_modifiers() {
    let ctx = context();
    let mut surface = EditorSurface::default();
    let mut document = document("    item");
    surface.bind_document(Some(&document), bookmark(4..4));
    frame(&mut surface, &mut document, &ctx, ViewMode::Edit, vec![]);
    // Shift was released after Tab, before this frame was delivered. The key
    // event retains SHIFT while frame's RawInput.modifiers is NONE.
    frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Edit,
        vec![egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: Some(egui::Key::Tab),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        }],
    );
    assert_eq!(document.content, "item");
    assert_eq!(surface.selection(&document), 0..0);
}

#[test]
fn full_audit_source_tab_preserves_reverse_selection_for_shift_end() {
    let ctx = context();
    let mut surface = EditorSurface::default();
    let mut document = document("one\ntwo");
    surface.bind_document(Some(&document), bookmark(0..7));
    surface.select_cursor(CCursorRange::two(CCursor::new(7), CCursor::new(0)));
    frame(&mut surface, &mut document, &ctx, ViewMode::Edit, vec![]);
    frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Edit,
        vec![egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: Some(egui::Key::Tab),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(document.content, "    one\n    two");
    frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Edit,
        vec![egui::Event::Key {
            key: egui::Key::End,
            physical_key: Some(egui::Key::End),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        }],
    );
    frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Edit,
        vec![egui::Event::Text("X".to_owned())],
    );
    assert_eq!(
        document.content, "    oneX",
        "Shift+End must move the reverse selection's first-line endpoint, not its last-line anchor"
    );
}

#[test]
fn full_audit_source_click_updates_the_surface_cursor_in_the_same_frame() {
    fn find_text(shape: &egui::Shape) -> Option<&egui::epaint::TextShape> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == "alpha beta" => Some(text),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(find_text),
            _ => None,
        }
    }
    let ctx = context();
    let mut surface = EditorSurface::default();
    let mut document = document("alpha beta");
    surface.bind_document(Some(&document), bookmark(10..10));
    let (output, _) = frame(&mut surface, &mut document, &ctx, ViewMode::Edit, vec![]);
    let text = output
        .shapes
        .iter()
        .find_map(|shape| find_text(&shape.shape))
        .unwrap();
    let position = text.pos
        + text
            .galley
            .pos_from_cursor(CCursor::new(2))
            .center()
            .to_vec2();
    frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Edit,
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert_eq!(surface.selection(&document), 2..2);
}

fn assert_tab_batch_matches_separate_frames(mode: ViewMode, outdent: bool) {
    let source = if outdent {
        "        one\n        two"
    } else {
        "one\ntwo"
    };
    let event = egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: Some(egui::Key::Tab),
        pressed: true,
        repeat: false,
        modifiers: if outdent {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        },
    };
    let run = |batched| {
        let ctx = context();
        let mut surface = EditorSurface::default();
        let mut document = document(source);
        surface.bind_document(Some(&document), bookmark(0..source.chars().count()));
        frame(&mut surface, &mut document, &ctx, mode, vec![]);
        if batched {
            frame(
                &mut surface,
                &mut document,
                &ctx,
                mode,
                vec![event.clone(), event.clone()],
            );
        } else {
            frame(&mut surface, &mut document, &ctx, mode, vec![event.clone()]);
            frame(&mut surface, &mut document, &ctx, mode, vec![event.clone()]);
        }
        let selection = surface.selection(&document);
        (document.content.clone(), selection)
    };
    let separate = run(false);
    let batched = run(true);
    assert_eq!(
        batched, separate,
        "{mode:?}, outdent={outdent}: event batching must not change text or selection"
    );
    assert_eq!(
        separate.0,
        if outdent {
            "one\ntwo"
        } else {
            "        one\n        two"
        },
        "Tab indentation must retain both selected lines"
    );
}

#[test]
fn full_audit_source_repeated_tabs_match_separate_frames() {
    assert_tab_batch_matches_separate_frames(ViewMode::Edit, false);
}

#[test]
fn full_audit_source_repeated_shift_tabs_match_separate_frames() {
    assert_tab_batch_matches_separate_frames(ViewMode::Edit, true);
}

#[test]
fn full_audit_hybrid_repeated_tabs_match_separate_frames() {
    assert_tab_batch_matches_separate_frames(ViewMode::Hybrid, false);
}

#[test]
fn full_audit_hybrid_repeated_shift_tabs_match_separate_frames() {
    assert_tab_batch_matches_separate_frames(ViewMode::Hybrid, true);
}

#[test]
fn full_audit_leading_tabs_leave_following_text_and_ime_events_in_order() {
    for barrier in [
        egui::Event::Text("X".to_owned()),
        egui::Event::Ime(egui::ImeEvent::Commit("中".to_owned())),
    ] {
        let ctx = context();
        let mut surface = EditorSurface::default();
        let mut document = document("line");
        surface.bind_document(Some(&document), bookmark(4..4));
        frame(&mut surface, &mut document, &ctx, ViewMode::Edit, vec![]);
        let tab = egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: Some(egui::Key::Tab),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut remaining = Vec::new();
        let mut expected_remaining = Vec::new();
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![tab.clone(), tab.clone(), barrier.clone(), tab.clone()],
                ..Default::default()
            },
            |ui| {
                // egui normalizes repeated key presses before exposing input to widgets.
                expected_remaining = ui.input(|input| input.events[2..].to_vec());
                assert!(surface.apply_leading_tab_input(ui, &mut document));
                remaining = ui.input(|input| input.events.clone());
            },
        );
        assert_eq!(document.content, "        line");
        assert_eq!(surface.selection(&document), 12..12);
        assert_eq!(remaining, expected_remaining);
    }
}

#[test]
fn binding_another_document_discards_deferred_undo_and_redo() {
    for redo in [false, true] {
        let ctx = context();
        let mut surface = EditorSurface::default();
        let mut first = document("first");
        let mut second = document("target");
        let before = second.content.clone();
        second.content.push('!');
        second.record_edit(before, Some(6..6), Some(7..7), EditKind::Other);
        if redo {
            second.undo().expect("prepare a real redo entry");
        }
        let expected = second.content.clone();
        surface.bind_document(Some(&first), bookmark(5..5));
        frame(&mut surface, &mut first, &ctx, ViewMode::Split, vec![]);
        surface.defer_history(redo);
        surface.bind_document(Some(&second), bookmark(0..0));

        let (output, _) = frame(&mut surface, &mut second, &ctx, ViewMode::Split, vec![]);
        assert_eq!(second.content, expected, "redo={redo}");
        assert_eq!(first.content, "first");
        assert_split_preview(&output, &expected);

        // The new document's history is intact; only the previous pending action was lost.
        surface.defer_history(redo);
        let (output, effects) = frame(&mut surface, &mut second, &ctx, ViewMode::Split, vec![]);
        let expected = if redo { "target!" } else { "target" };
        assert_eq!(second.content, expected);
        assert_eq!(effects.notice, if redo { "已重做" } else { "已撤销" });
        assert_split_preview(&output, expected);
    }
}

#[test]
fn switching_during_ime_does_not_commit_or_restore_the_old_composition() {
    let ctx = context();
    let mut surface = EditorSurface::default();
    let mut first = document("AB");
    let mut second = document("CD");
    surface.bind_document(Some(&first), bookmark(1..1));
    frame(&mut surface, &mut first, &ctx, ViewMode::Hybrid, vec![]);
    let (preedit, _) = frame(
        &mut surface,
        &mut first,
        &ctx,
        ViewMode::Hybrid,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "ni".to_owned(),
            active_range_chars: Some(0..2),
        })],
    );
    assert_eq!(first.content, "AB");
    assert!(painted_text(&preedit, 0.0).contains("ni"));

    surface.bind_document(Some(&second), bookmark(1..1));
    frame(&mut surface, &mut second, &ctx, ViewMode::Hybrid, vec![]);
    frame(
        &mut surface,
        &mut second,
        &ctx,
        ViewMode::Hybrid,
        vec![egui::Event::Text("X".to_owned())],
    );
    assert_eq!(second.content, "CXD");
    assert_eq!(first.content, "AB");

    surface.bind_document(Some(&first), bookmark(1..1));
    let (returned, _) = frame(&mut surface, &mut first, &ctx, ViewMode::Hybrid, vec![]);
    assert!(!painted_text(&returned, 0.0).contains("ni"));
    frame(
        &mut surface,
        &mut first,
        &ctx,
        ViewMode::Hybrid,
        vec![egui::Event::Text("🙂".to_owned())],
    );
    assert_eq!(first.content, "A🙂B");
    assert_eq!(second.content, "CXD");
}

#[test]
fn a_committed_command_cancels_old_composition_before_following_input() {
    let ctx = context();
    let mut surface = EditorSurface::default();
    let mut document = document("AB");
    surface.bind_document(Some(&document), bookmark(1..1));
    frame(&mut surface, &mut document, &ctx, ViewMode::Hybrid, vec![]);
    frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Hybrid,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "ni".to_owned(),
            active_range_chars: Some(0..2),
        })],
    );
    document.edit(EditKind::Format, None, |text| {
        *text = "新文🦀".to_owned();
        Some(3..3)
    });
    surface.document_edited(&document, Some(3..3));
    let (painted, _) = frame(&mut surface, &mut document, &ctx, ViewMode::Hybrid, vec![]);
    assert!(!painted_text(&painted, 0.0).contains("ni"));
    frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Hybrid,
        vec![egui::Event::Text("🙂".to_owned())],
    );
    assert_eq!(document.content, "新文🦀🙂");
    document.undo().unwrap();
    assert_eq!(document.content, "新文🦀");
    document.undo().unwrap();
    assert_eq!(document.content, "AB");
}

#[test]
fn a_committed_command_replaces_a_cross_block_selection_with_its_new_cursor() {
    let ctx = context();
    let mut surface = EditorSurface::default();
    let mut document = document("FIRST\n\nSECOND");
    surface.bind_document(Some(&document), bookmark(1..10));
    frame(&mut surface, &mut document, &ctx, ViewMode::Hybrid, vec![]);
    document.edit(EditKind::Other, None, |text| {
        *text = "新正文".to_owned();
        Some(1..1)
    });
    surface.document_edited(&document, Some(1..1));
    frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Hybrid,
        vec![egui::Event::Text("🙂".to_owned())],
    );
    assert_eq!(document.content, "新🙂正文");
}

#[test]
fn switching_after_a_cross_block_selection_types_at_the_new_document_cursor() {
    let ctx = context();
    let mut surface = EditorSurface::default();
    let mut first = document("FIRST\n\nSECOND");
    let mut second = document("target");
    surface.bind_document(Some(&first), bookmark(1..10));
    frame(&mut surface, &mut first, &ctx, ViewMode::Hybrid, vec![]);
    let (copied, _) = frame(
        &mut surface,
        &mut first,
        &ctx,
        ViewMode::Hybrid,
        vec![egui::Event::Copy],
    );
    assert!(copied.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == "IRST\n\nSEC")
    }));

    surface.bind_document(Some(&second), bookmark(2..2));
    frame(
        &mut surface,
        &mut second,
        &ctx,
        ViewMode::Hybrid,
        vec![egui::Event::Text("X".to_owned())],
    );
    assert_eq!(second.content, "taXrget");
    assert_eq!(first.content, "FIRST\n\nSECOND");

    surface.bind_document(Some(&first), bookmark(1..1));
    frame(
        &mut surface,
        &mut first,
        &ctx,
        ViewMode::Hybrid,
        vec![egui::Event::Text("Z".to_owned())],
    );
    assert_eq!(first.content, "FZIRST\n\nSECOND");
}

#[test]
fn remembered_views_follow_identity_and_clamp_to_unicode_characters_after_shrinking() {
    let ctx = context();
    let mut surface = EditorSurface::default();
    let mut first = document("中🙂abcdef");
    let mut second = document("β🙂末");
    assert_eq!(first.title(), second.title());
    surface.bind_document(Some(&first), bookmark(4..8));
    frame(&mut surface, &mut first, &ctx, ViewMode::Edit, vec![]);
    surface.remember(first.id());
    surface.bind_document(Some(&second), bookmark(1..2));
    frame(&mut surface, &mut second, &ctx, ViewMode::Edit, vec![]);
    surface.remember(second.id());

    first.content = "中🙂文".to_owned();
    first.update_after_edit();
    surface.bind_document(Some(&first), bookmark(0..0));
    frame(
        &mut surface,
        &mut first,
        &ctx,
        ViewMode::Edit,
        vec![egui::Event::Text("X".to_owned())],
    );
    assert_eq!(first.content, "中🙂文X");

    surface.bind_document(Some(&second), bookmark(0..0));
    frame(
        &mut surface,
        &mut second,
        &ctx,
        ViewMode::Edit,
        vec![egui::Event::Text("R".to_owned())],
    );
    assert_eq!(second.content, "βR末");
}

#[test]
fn forgetting_a_view_restores_the_supplied_bookmark_instead_of_its_saved_selection() {
    let ctx = context();
    let mut surface = EditorSurface::default();
    let mut document = document("中🙂tail");
    surface.bind_document(Some(&document), bookmark(2..6));
    frame(&mut surface, &mut document, &ctx, ViewMode::Edit, vec![]);
    surface.remember(document.id());
    surface.queue_editor_selection(1..2);
    frame(&mut surface, &mut document, &ctx, ViewMode::Edit, vec![]);
    let fallback = surface.bookmark();
    surface.bind_document(None, EditorBookmark::default());
    surface.forget(document.id());
    surface.bind_document(Some(&document), fallback);
    frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Edit,
        vec![egui::Event::Text("新".to_owned())],
    );
    assert_eq!(document.content, "中新tail");
}

#[test]
fn split_preview_observes_same_frame_input_and_deferred_history_in_order() {
    let ctx = context();
    let mut surface = EditorSurface::default();
    let mut document = document("BASE");
    surface.bind_document(Some(&document), bookmark(4..4));
    frame(&mut surface, &mut document, &ctx, ViewMode::Split, vec![]);

    surface.defer_history(false);
    let (output, effects) = frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Split,
        vec![egui::Event::Text("X".to_owned())],
    );
    assert_eq!(document.content, "BASE");
    assert_eq!(effects.notice, "已撤销");
    assert_split_preview(&output, "BASE");

    surface.defer_history(true);
    let (output, effects) = frame(&mut surface, &mut document, &ctx, ViewMode::Split, vec![]);
    assert_eq!(document.content, "BASEX");
    assert_eq!(effects.notice, "已重做");
    assert_split_preview(&output, "BASEX");

    surface.defer_history(false);
    frame(&mut surface, &mut document, &ctx, ViewMode::Split, vec![]);
    assert_eq!(document.content, "BASE");
    surface.defer_history(true);
    let (output, effects) = frame(
        &mut surface,
        &mut document,
        &ctx,
        ViewMode::Split,
        vec![egui::Event::Text("Y".to_owned())],
    );
    assert_eq!(document.content, "BASEY");
    assert_eq!(effects.notice, "没有可重做的操作");
    assert_split_preview(&output, "BASEY");
}
