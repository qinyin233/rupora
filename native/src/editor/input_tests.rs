//! Source-coordinate input, projection, and composition regressions.

use super::*;
use eframe::egui::Context;

#[test]
fn calculates_safe_split_scroll_ratios() {
    assert_eq!(
        scroll_ratio(PaneScroll {
            offset: 50.0,
            maximum: 100.0,
            hovered: true,
        }),
        0.5
    );
    assert_eq!(scroll_ratio(PaneScroll::default()), 0.0);
}

#[test]
fn cross_block_selection_treats_fenced_code_as_an_atomic_block() {
    let source = "before α\n\n```rust\n代码();\n```\n\nafter β";
    let blocks = markdown::blocks(source);
    assert_eq!(blocks.len(), 3);
    let before = &blocks[0];
    let code = &blocks[1];
    let after = &blocks[2];
    let char_at = |byte| source[..byte].chars().count();

    let forward = snap_atomic_cross_block_selection(
        source,
        &blocks,
        before.id,
        char_at(before.range.end) - 2,
        code.id,
        char_at(code.range.start) + 4,
    );
    assert_eq!(
        forward.primary.index.0,
        char_at(code.range.end),
        "the whole code block must be selected at the upper edge"
    );

    let backward = snap_atomic_cross_block_selection(
        source,
        &blocks,
        code.id,
        char_at(code.range.start) + 5,
        before.id,
        char_at(before.range.end) - 2,
    );
    assert_eq!(
        backward.secondary.index.0,
        char_at(code.range.end),
        "a later code anchor must snap to its far edge"
    );

    let from_code = snap_atomic_cross_block_selection(
        source,
        &blocks,
        code.id,
        char_at(code.range.start) + 5,
        after.id,
        char_at(after.range.start) + 2,
    );
    assert_eq!(from_code.secondary.index.0, char_at(code.range.start));
}

#[test]
fn removing_a_code_block_also_removes_one_structural_gap() {
    for (source, index, expected) in [
        ("```\na\n```\n\nafter", 0, "after"),
        ("before\n\n```\na\n```", 1, "before"),
        ("```\na\n```", 0, ""),
        ("```\n\n```\n\n", 0, ""),
    ] {
        let blocks = markdown::blocks(source);
        let range = code_block_removal_range(source, &blocks, index);
        let mut updated = source.to_owned();
        updated.replace_range(range, "");
        assert_eq!(updated, expected);
    }
}

#[test]
fn double_click_after_code_targets_a_real_plain_paragraph() {
    let source = "```\ncode\n```";
    let blocks = markdown::blocks(source);
    let (range, replacement, cursor) = paragraph_after_code_double_click(source, &blocks, 0);
    assert_eq!(range, 0..source.len());
    assert_eq!(replacement.as_deref(), Some("```\ncode\n```\n\n"));
    assert_eq!(cursor, "```\ncode\n```\n\n".chars().count());

    let source = "```\ncode\n```\n\nnext";
    let blocks = markdown::blocks(source);
    let (range, replacement, cursor) = paragraph_after_code_double_click(source, &blocks, 0);
    assert_eq!(range.start, range.end);
    assert!(replacement.is_none());
    assert_eq!(cursor, source.find("next").unwrap());
}

#[test]
fn wysiwyg_editor_excludes_paragraph_separators_from_the_active_range() {
    let source = "第一段\n\n第二段";
    let blocks = markdown::blocks(source);
    let first_range = hybrid_edit_range(source, &blocks, blocks[0].id);
    assert_eq!(&source[first_range], "第一段");

    let mut trailing = "换句话".to_owned();
    let original_blocks = markdown::blocks(&trailing);
    let original_range = hybrid_edit_range(&trailing, &original_blocks, original_blocks[0].id);
    let replacement = format!("{}\n", &trailing[original_range.clone()]);
    trailing.replace_range(original_range, &replacement);

    let updated_blocks = markdown::blocks(&trailing);
    let updated_range = hybrid_edit_range(&trailing, &updated_blocks, updated_blocks[0].id);
    assert_eq!(&trailing[updated_range], "换句话");
}

#[test]
fn hidden_inline_code_boundaries_preserve_the_outside_cursor_side() {
    let source = "`abc`";
    let projection = VisualProjection::from_markdown(source);
    let previous_source = 5..5;
    let previous_visual = projection.visual_char_range(source, previous_source.clone());
    assert_eq!(previous_visual, 3..3);

    assert_eq!(
        source_selection_after_visual_input(
            &projection,
            source,
            Some(&previous_source),
            Some(&previous_visual),
            previous_visual.clone(),
        ),
        previous_source
    );
    assert_eq!(
        source_selection_after_visual_input(
            &projection,
            source,
            Some(&(5..5)),
            Some(&(3..3)),
            2..2,
        ),
        3..3
    );

    let changed_source = "x";
    let changed_projection = VisualProjection::from_markdown(changed_source);
    assert_eq!(
        source_selection_after_visual_input(
            &changed_projection,
            changed_source,
            Some(&(99..99)),
            Some(&(0..0)),
            0..0,
        ),
        0..0
    );
}

#[test]
fn cursor_localization_saturates_both_selection_ends() {
    let range = CCursorRange::two(CCursor::new(1), CCursor::new(3));
    let localized = cursor_range_saturating_sub(range, 10);
    assert_eq!(localized.primary.index.0, 0);
    assert_eq!(localized.secondary.index.0, 0);
}

#[test]
fn source_code_editor_enter_continues_one_nested_list_prefix() {
    use egui::{Event, Id, Modifiers, RawInput};

    let context = Context::default();
    let id = Id::new("source-list-enter-regression");
    let mut source = "  - item".to_owned();
    let mut cursor = None;
    let _ = context.run_ui(
        RawInput {
            events: vec![Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..RawInput::default()
        },
        |ui| {
            let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(
                    source.chars().count(),
                ))));
            state.store(ui.ctx(), id);
            ui.memory_mut(|memory| memory.request_focus(id));
            let output = TextEdit::multiline(&mut source)
                .id(id)
                .code_editor()
                .show(ui);
            cursor = output
                .cursor_range
                .map(cursor_range_to_char_range)
                .map(|range| range.end);
        },
    );

    assert!(
        matches!(source.as_str(), "  - item\n" | "  - item\n  "),
        "egui may change its code-editor auto-indent policy between releases: {source:?}"
    );
    let cursor = cursor.expect("the focused source editor should retain its cursor");
    let selection = editing::continue_markdown_line(&mut source, cursor).unwrap();
    assert_eq!(source, "  - item\n  - ");
    assert_eq!(selection.end, source.chars().count());
}

#[test]
fn wysiwyg_reads_click_and_drag_selection_from_the_post_pointer_state() {
    use egui::{Event, Id, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

    fn run_frame(
        context: &Context,
        id: Id,
        text: &mut String,
        events: Vec<Event>,
        request_focus: bool,
    ) -> (Arc<egui::Galley>, egui::Pos2, Option<CCursorRange>) {
        let mut result = None;
        let _ = context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 160.0))),
                events,
                ..RawInput::default()
            },
            |ui| {
                ui.set_width(480.0);
                if request_focus {
                    ui.memory_mut(|memory| memory.request_focus(id));
                }
                let output = TextEdit::singleline(text).id(id).show(ui);
                result = Some((
                    output.galley.clone(),
                    output.galley_pos,
                    text_edit_cursor_after_input(&output),
                ));
            },
        );
        result.expect("text edit should be laid out")
    }

    fn cursor_position(galley: &egui::Galley, galley_pos: egui::Pos2, index: usize) -> egui::Pos2 {
        galley_pos
            + galley
                .pos_from_cursor(CCursor::new(index))
                .center()
                .to_vec2()
    }

    let context = Context::default();
    let id = Id::new("wysiwyg-pointer-selection-regression");
    let mut text = "alpha beta 中文".to_owned();
    let (galley, galley_pos, _) = run_frame(&context, id, &mut text, Vec::new(), true);
    let click = cursor_position(&galley, galley_pos, 2);
    let (_, _, clicked) = run_frame(
        &context,
        id,
        &mut text,
        vec![
            Event::PointerMoved(click),
            Event::PointerButton {
                pos: click,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        false,
    );
    assert_eq!(clicked.map(cursor_range_to_char_range), Some(2..2));

    let (galley, galley_pos, _) = run_frame(&context, id, &mut text, Vec::new(), false);
    let drag_to = cursor_position(&galley, galley_pos, 10);
    let (_, _, selected) = run_frame(
        &context,
        id,
        &mut text,
        vec![Event::PointerMoved(drag_to)],
        false,
    );
    let selected = selected.expect("dragging should create a selection");
    assert_eq!(cursor_range_to_char_range(selected), 2..10);
    assert_eq!(selected.primary.index.0, 10);
    assert_eq!(selected.secondary.index.0, 2);

    let (galley, galley_pos, _) = run_frame(
        &context,
        id,
        &mut text,
        vec![Event::PointerButton {
            pos: drag_to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        false,
    );

    let reverse_start = cursor_position(&galley, galley_pos, 12);
    let (_, _, reverse_anchor) = run_frame(
        &context,
        id,
        &mut text,
        vec![
            Event::PointerMoved(reverse_start),
            Event::PointerButton {
                pos: reverse_start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        false,
    );
    assert_eq!(reverse_anchor.map(cursor_range_to_char_range), Some(12..12));

    let (galley, galley_pos, _) = run_frame(&context, id, &mut text, Vec::new(), false);
    let reverse_to = cursor_position(&galley, galley_pos, 6);
    let (_, _, reverse_selected) = run_frame(
        &context,
        id,
        &mut text,
        vec![Event::PointerMoved(reverse_to)],
        false,
    );
    let reverse_selected = reverse_selected.expect("reverse dragging should select text");
    assert_eq!(cursor_range_to_char_range(reverse_selected), 6..12);
    assert_eq!(reverse_selected.primary.index.0, 6);
    assert_eq!(reverse_selected.secondary.index.0, 12);

    let reverse =
        cursor_range_with_direction(2..10, CCursorRange::two(CCursor::new(10), CCursor::new(2)));
    assert_eq!(reverse.primary.index.0, 2);
    assert_eq!(reverse.secondary.index.0, 10);
}

#[test]
fn wysiwyg_text_edit_keeps_source_cursor_when_markers_become_hidden() {
    use egui::{Event, Id, Modifiers, RawInput};

    fn edit_frame(
        context: &Context,
        id: Id,
        source: &str,
        source_selection: std::ops::Range<usize>,
        events: Vec<Event>,
    ) -> crate::wysiwyg::VisualSourceEdit {
        let projection =
            VisualProjection::from_markdown_with_selection(source, Some(source_selection.clone()));
        let visual_selection = projection.visual_char_range(source, source_selection);
        let mut visual_content = projection.text().to_owned();
        let mut result = None;
        let _ = context.run_ui(
            RawInput {
                events,
                ..RawInput::default()
            },
            |ui| {
                let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(CCursor::new(
                        visual_selection.start,
                    ))));
                state.store(ui.ctx(), id);
                ui.memory_mut(|memory| memory.request_focus(id));
                let output = TextEdit::multiline(&mut visual_content)
                    .id(id)
                    .hint_text("开始写作…")
                    .show(ui);
                let visual_cursor = text_edit_cursor_after_input(&output)
                    .map(cursor_range_to_char_range)
                    .expect("focused TextEdit should retain a cursor");
                result = projection.apply_edit(source, &visual_content, visual_cursor);
            },
        );
        result.expect("typed frame should update the source")
    }

    fn type_frame(
        context: &Context,
        id: Id,
        source: &str,
        source_selection: std::ops::Range<usize>,
        text: &str,
    ) -> crate::wysiwyg::VisualSourceEdit {
        edit_frame(
            context,
            id,
            source,
            source_selection,
            vec![Event::Text(text.to_owned())],
        )
    }

    let context = Context::default();
    let id = Id::new("wysiwyg-hidden-marker-cursor-regression");
    let mut update = type_frame(&context, id, "", 0..0, "#");
    assert_eq!(update.source, "#");
    update = type_frame(&context, id, &update.source, update.selection, " ");
    assert_eq!(update.source, "# ");
    update = type_frame(&context, id, &update.source, update.selection, "ATX 标题");
    assert_eq!(update.source, "# ATX 标题");

    update = edit_frame(
        &context,
        id,
        &update.source,
        update.selection,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    update.selection =
        crate::wysiwyg::complete_visual_enter(&mut update.source, update.selection, false);
    assert_eq!(update.source, "# ATX 标题\n\n");
    update = type_frame(&context, id, &update.source, update.selection, "下一行");
    assert_eq!(update.source, "# ATX 标题\n\n下一行");
}

#[test]
fn block_boundary_backspace_can_remove_one_blank_line_at_a_time() {
    let mut source = "`a\n\n\n`".to_owned();
    for expected in ["`a\n\n`", "`a\n`"] {
        let blocks = markdown::blocks(&source);
        let closing = source.chars().count() - 1;
        let block = block_for_char_index(&source, &blocks, closing);
        let edit_range = hybrid_edit_range(&source, &blocks, block.id);
        assert_eq!(&source[edit_range.clone()], "`");

        let original_block = source[edit_range.clone()].to_owned();
        let (replacement_range, cursor) = boundary_backspace_edit(&source, edit_range).unwrap();
        source.replace_range(replacement_range, &original_block);
        assert_eq!(source, expected);
        assert_eq!(cursor, expected.chars().count() - 1);
    }

    assert_eq!(line_break_before("one\r\ntwo", 5), Some(3..5));
    assert_eq!(line_break_before("one two", 4), None);
}

#[test]
fn paragraph_boundary_backspace_removes_each_extra_blank_line() {
    let mut source = "first\n\n\n\nsecond".to_owned();
    for expected in ["first\n\n\nsecond", "first\n\nsecond"] {
        let blocks = markdown::blocks(&source);
        let second_range = hybrid_edit_range(&source, &blocks, blocks.last().unwrap().id);
        let second = source[second_range.clone()].to_owned();
        let (replacement, cursor) = boundary_backspace_edit(&source, second_range).unwrap();
        source.replace_range(replacement, &second);
        assert_eq!(source, expected);
        assert_eq!(cursor, expected.find("second").unwrap());
    }
}

#[test]
fn wysiwyg_empty_paragraphs_have_independent_source_positions() {
    for (source, empty) in [
        ("第一段\n\n第二段", 0),
        ("第一段\n\n\n第二段", 1),
        ("第一段\n\n\n\n第二段", 1),
        ("第一段\n\n\n\n\n\n第二段", 2),
    ] {
        let blocks = markdown::blocks(source);
        assert_eq!(
            blocks.iter().filter(|block| block.range.is_empty()).count(),
            empty
        );
        assert_eq!(
            &source[hybrid_edit_range(source, &blocks, blocks[0].id)],
            "第一段"
        );
        for block in blocks.iter().filter(|block| block.range.is_empty()) {
            let cursor = source[..block.range.start].chars().count();
            assert_eq!(block_for_char_index(source, &blocks, cursor).id, block.id);
        }
    }
    let source = "first\r\n\r\n\r\n\r\nsecond";
    let blocks = markdown::blocks(source);
    assert_eq!(
        blocks[1].range,
        "first\r\n\r\n".len().."first\r\n\r\n".len()
    );
    assert_eq!(&source[blocks[2].range.clone()], "second");
}

#[test]
fn classifies_ime_preedit_commit_and_cancel_frames() {
    use egui::{Event, ImeEvent};

    assert_eq!(
        ime_frame_action(&[Event::Ime(ImeEvent::Preedit {
            text: "ni".to_owned(),
            active_range_chars: Some(0..2),
        })]),
        ImeFrameAction::Preedit
    );
    assert_eq!(
        ime_frame_action(&[Event::Ime(ImeEvent::Commit("你".to_owned()))]),
        ImeFrameAction::Commit
    );
    assert_eq!(
        ime_frame_action(&[Event::Ime(ImeEvent::Preedit {
            text: String::new(),
            active_range_chars: None,
        })]),
        ImeFrameAction::Cancel
    );
}

#[test]
fn wysiwyg_ime_session_preserves_preedit_text_between_frames() {
    use egui::{Event, Id, ImeEvent, RawInput};

    fn run_frame(context: &Context, text: &mut String, events: Vec<Event>, request_focus: bool) {
        let _ = context.run_ui(
            RawInput {
                events,
                ..RawInput::default()
            },
            |ui| {
                let id = Id::new("wysiwyg-ime-regression");
                if request_focus {
                    ui.memory_mut(|memory| memory.request_focus(id));
                }
                TextEdit::multiline(text).id(id).show(ui);
            },
        );
    }

    let context = Context::default();
    let block_id = markdown::blocks("")[0].id;
    let mut session = HybridImeSession {
        document_id: 1,
        block_id,
        base_source: String::new(),
        visual_content: String::new(),
    };

    run_frame(&context, &mut session.visual_content, Vec::new(), true);
    run_frame(
        &context,
        &mut session.visual_content,
        vec![Event::Ime(ImeEvent::Preedit {
            text: "n".to_owned(),
            active_range_chars: Some(0..1),
        })],
        false,
    );
    assert_eq!(session.visual_content, "n");

    run_frame(
        &context,
        &mut session.visual_content,
        vec![Event::Ime(ImeEvent::Preedit {
            text: "ni".to_owned(),
            active_range_chars: Some(0..2),
        })],
        false,
    );
    assert_eq!(session.visual_content, "ni");

    run_frame(
        &context,
        &mut session.visual_content,
        vec![Event::Ime(ImeEvent::Commit("你".to_owned()))],
        false,
    );
    assert_eq!(session.visual_content, "你");
}

#[test]
fn wysiwyg_third_backtick_accepts_code_on_the_very_next_frame() {
    use egui::{Event, Id, Modifiers, RawInput};

    fn edit_frame(
        context: &Context,
        id: Id,
        source: &str,
        source_selection: std::ops::Range<usize>,
        events: Vec<Event>,
    ) -> crate::wysiwyg::VisualSourceEdit {
        let projection =
            VisualProjection::from_markdown_with_selection(source, Some(source_selection.clone()));
        let visual_selection = projection.visual_char_range(source, source_selection);
        let mut visual_content = projection.text().to_owned();
        let mut result = None;
        let _ = context.run_ui(
            RawInput {
                events,
                ..RawInput::default()
            },
            |ui| {
                let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(CCursor::new(
                        visual_selection.start,
                    ))));
                state.store(ui.ctx(), id);
                ui.memory_mut(|memory| memory.request_focus(id));
                let mut editor = TextEdit::multiline(&mut visual_content).id(id);
                if is_fenced_code_block(source) {
                    editor = editor.code_editor();
                }
                let output = editor.show(ui);
                let visual_cursor = text_edit_cursor_after_input(&output)
                    .map(cursor_range_to_char_range)
                    .expect("focused editor should retain a cursor");
                result = projection.apply_edit(source, &visual_content, visual_cursor);
            },
        );
        result.expect("typing should update the projected source")
    }

    fn type_frame(
        context: &Context,
        id: Id,
        source: &str,
        source_selection: std::ops::Range<usize>,
        text: &str,
    ) -> crate::wysiwyg::VisualSourceEdit {
        edit_frame(
            context,
            id,
            source,
            source_selection,
            vec![Event::Text(text.to_owned())],
        )
    }

    let context = Context::default();
    let id = Id::new("wysiwyg-fence-first-code-regression");
    let mut update = crate::wysiwyg::VisualSourceEdit {
        source: String::new(),
        selection: 0..0,
    };
    for marker in ["`", "`", "`"] {
        update = type_frame(&context, id, &update.source, update.selection, marker);
        if let Some(selection) =
            complete_bare_fenced_code_after_typing(&mut update.source, update.selection.clone())
        {
            update.selection = selection;
        }
    }
    assert_eq!(update.source, "```\n\n```");
    assert_eq!(update.selection, 4..4);

    update = type_frame(
        &context,
        id,
        &update.source,
        update.selection,
        "fn main() {} 中文",
    );
    assert_eq!(update.source, "```\nfn main() {} 中文\n```");
    assert_eq!(
        VisualProjection::from_markdown(&update.source).text(),
        "fn main() {} 中文"
    );

    update = edit_frame(
        &context,
        id,
        &update.source,
        update.selection,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    for marker in ["`", "`", "`"] {
        update = type_frame(&context, id, &update.source, update.selection, marker);
        if let Some(selection) =
            consume_paired_fenced_code_closer(&mut update.source, update.selection.clone())
        {
            update.selection = selection;
        } else if let Some(selection) =
            complete_bare_fenced_code_after_typing(&mut update.source, update.selection.clone())
        {
            update.selection = selection;
        }
    }
    assert_eq!(update.source, "```\nfn main() {} 中文\n```\n\n");
    assert_eq!(update.selection.end, update.source.chars().count());

    update = type_frame(&context, id, &update.source, update.selection, "AFTER");
    assert_eq!(update.source, "```\nfn main() {} 中文\n```\n\nAFTER");
}

#[test]
fn cross_block_input_is_consumed_before_the_active_widget_can_duplicate_it() {
    use egui::{Event, RawInput};

    let context = Context::default();
    let mut action = None;
    let mut remaining = usize::MAX;
    let _ = context.run_ui(
        RawInput {
            events: vec![Event::Text("替换".to_owned())],
            ..RawInput::default()
        },
        |ui| {
            action = take_cross_block_input(ui);
            remaining = ui.input(|input| input.events.len());
        },
    );
    assert!(matches!(action, Some(CrossBlockInput::Replace(ref text)) if text == "替换"));
    assert_eq!(remaining, 0);

    let selected = "前段\n\n```\ncode\n```";
    let range = clamp_char_range(selected, 2..selected.chars().count());
    let start = char_to_byte(selected, range.start);
    let end = char_to_byte(selected, range.end);
    let mut replaced = selected.to_owned();
    replaced.replace_range(start..end, "新");
    assert_eq!(replaced, "前段新");
}

#[test]
fn product_input_newline_code_exit_reuses_empty_paragraph_before_following_text() {
    let source = "```\nCODE\n```\n\n\n\nNEXT";
    let blocks = markdown::blocks(source);
    let (_, replacement, cursor) = paragraph_after_code_double_click(source, &blocks, 0);
    assert!(replacement.is_none());
    assert_eq!(cursor, blocks[1].range.start);
}
