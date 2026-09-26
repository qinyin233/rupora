use super::*;

#[test]
fn source_ime_preedit_preserves_document_and_cancelled_selection() {
    for selection in [1..1, 1..3] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), "A中文🙂B", selection);
        let ctx = Context::default();
        frame(&mut app, &ctx, false, vec![]);
        let token = app.session[0].snapshot_token();
        for text in ["n", "ni"] {
            frame(
                &mut app,
                &ctx,
                false,
                vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: text.into(),
                    active_range_chars: Some(0..text.len()),
                })],
            );
            assert_eq!(app.session[0].content, "A中文🙂B");
            assert_eq!(app.session[0].snapshot_token(), token);
            assert!(!app.session[0].can_undo());
        }
        frame(
            &mut app,
            &ctx,
            false,
            vec![
                egui::Event::Ime(egui::ImeEvent::Commit(String::new())),
                egui::Event::Text("末".into()),
            ],
        );
        assert_eq!(app.session[0].content, "A末中文🙂B");
        app.undo_active();
        assert_eq!(app.session[0].content, "A中文🙂B");
    }
}

#[test]
fn source_ime_mode_switch_does_not_publish_preedit() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), "A🙂B", 1..1);
    app.state.view_mode = ViewMode::Edit;
    let ctx = Context::default();
    frame(&mut app, &ctx, false, vec![]);
    frame(
        &mut app,
        &ctx,
        false,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "ni".into(),
            active_range_chars: Some(0..2),
        })],
    );
    app.execute(AppCommand::SetView(ViewMode::Hybrid));
    frame(&mut app, &ctx, true, vec![]);
    assert_eq!(app.session[0].content, "A🙂B");
    app.execute(AppCommand::SetView(ViewMode::Edit));
    frame(&mut app, &ctx, false, vec![]);
    frame(&mut app, &ctx, false, vec![egui::Event::Text("末".into())]);
    assert_eq!(app.session[0].content, "A末🙂B");
}

#[test]
fn source_ime_cancel_keeps_plain_text_before_preedit_in_the_same_frame() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), "A🙂B", 1..1);
    let ctx = Context::default();
    frame(&mut app, &ctx, false, vec![]);
    frame(
        &mut app,
        &ctx,
        false,
        vec![
            egui::Event::Text("X".into()),
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "ni".into(),
                active_range_chars: Some(0..2),
            }),
        ],
    );
    frame(
        &mut app,
        &ctx,
        false,
        vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new()))],
    );
    assert_eq!(app.session[0].content, "AX🙂B");
}

#[test]
fn source_ime_literal_input_interrupts_preedit_without_losing_the_edit() {
    for event in [
        egui::Event::Text("X".into()),
        egui::Event::Paste("X".into()),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), "A中文🙂B", 1..3);
        let ctx = Context::default();
        frame(&mut app, &ctx, false, vec![]);
        frame(
            &mut app,
            &ctx,
            false,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "ni".into(),
                active_range_chars: Some(0..2),
            })],
        );
        frame(&mut app, &ctx, false, vec![event]);
        frame(
            &mut app,
            &ctx,
            false,
            vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new()))],
        );
        assert_eq!(app.session[0].content, "AX🙂B");
        app.undo_active();
        assert_eq!(app.session[0].content, "A中文🙂B");
    }
}

#[test]
fn source_ime_commit_after_candidate_pause_undoes_the_whole_composition() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), "A🙂B", 1..1);
    let ctx = Context::default();
    frame(&mut app, &ctx, false, vec![]);
    for text in ["z", "zhong'wen"] {
        frame(
            &mut app,
            &ctx,
            false,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: text.into(),
                active_range_chars: Some(0..text.chars().count()),
            })],
        );
    }
    // Choosing a candidate can exceed the ordinary 900 ms typing coalescing
    // window. That pause must not turn provisional spelling into undo history.
    std::thread::sleep(std::time::Duration::from_millis(1000));
    frame(
        &mut app,
        &ctx,
        false,
        vec![
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: String::new(),
                active_range_chars: None,
            }),
            egui::Event::Ime(egui::ImeEvent::Commit("中文".into())),
        ],
    );
    assert_eq!(app.session[0].content, "A中文🙂B");
    app.undo_active();
    assert_eq!(app.session[0].content, "A🙂B");
    assert_eq!(app.active_selection(0), 1..1);
    app.redo_active();
    assert_eq!(app.session[0].content, "A中文🙂B");
}

#[test]
fn source_ime_ordinary_keys_interrupt_preedit_and_keep_their_edit() {
    for (pressed, expected) in [
        (Key::Enter, "A\n🙂B"),
        (Key::Backspace, "A🙂B"),
        (Key::Delete, "A🙂B"),
        (Key::Tab, "    A中文🙂B"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), "A中文🙂B", 1..3);
        let ctx = Context::default();
        frame(&mut app, &ctx, false, vec![]);
        frame(
            &mut app,
            &ctx,
            false,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "ni".into(),
                active_range_chars: Some(0..2),
            })],
        );
        frame(
            &mut app,
            &ctx,
            false,
            vec![key(pressed, egui::Modifiers::NONE)],
        );
        frame(
            &mut app,
            &ctx,
            false,
            vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new()))],
        );
        assert_eq!(app.session[0].content, expected, "{pressed:?}");
        app.undo_active();
        assert_eq!(app.session[0].content, "A中文🙂B");
    }
}

#[test]
fn ime_clear_before_commit_replaces_the_original_selection() {
    for hybrid in [false, true] {
        for (selection, expected) in [(2..4, "A中新B"), (2..2, "A中新文🙂B")] {
            for trailing_clear in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let mut app = app_at(directory.path(), "A中文🙂B", selection.clone());
                let ctx = Context::default();
                frame(&mut app, &ctx, hybrid, vec![]);
                for text in ["x", "xi", "xin"] {
                    frame(
                        &mut app,
                        &ctx,
                        hybrid,
                        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                            text: text.into(),
                            active_range_chars: Some(0..text.len()),
                        })],
                    );
                    if hybrid {
                        assert_eq!(app.session[0].content, "A中文🙂B");
                    }
                    frame(&mut app, &ctx, hybrid, vec![]);
                }
                // winit's Windows WM_IME_COMPOSITION / WM_IME_ENDCOMPOSITION
                // paths clear the preedit immediately before the candidate.
                let clear = egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: String::new(),
                    active_range_chars: None,
                });
                let mut events = vec![
                    clear.clone(),
                    egui::Event::Ime(egui::ImeEvent::Commit("新".into())),
                ];
                if trailing_clear {
                    events.push(clear);
                }
                frame(&mut app, &ctx, hybrid, events);
                assert_eq!(
                    app.session[0].content, expected,
                    "hybrid={hybrid}, selection={selection:?}, trailing_clear={trailing_clear}"
                );
                app.undo_active();
                assert_eq!(app.session[0].content, "A中文🙂B");
                app.redo_active();
                assert_eq!(app.session[0].content, expected);
            }
        }
    }
}

#[test]
fn replacing_image_alt_and_adjacent_format_in_one_frame_keeps_valid_markup() {
    let source = "![**甲**](a)*乙*";
    let directory = tempfile::tempdir().unwrap();
    let end = source.chars().count();
    let mut app = app_at(directory.path(), source, end..end);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    frame(
        &mut app,
        &ctx,
        true,
        vec![
            key(Key::ArrowLeft, egui::Modifiers::SHIFT),
            key(Key::ArrowLeft, egui::Modifiers::SHIFT),
            egui::Event::Text("新".into()),
        ],
    );
    assert_eq!(
        markdown::render_html_fragment(&app.session[0].content),
        "<p><img src=\"a\" alt=\"新\"></p>\n",
        "{}",
        app.session[0].content,
    );
    app.undo_active();
    assert_eq!(app.session[0].content, source);
}

#[test]
fn hybrid_deleting_first_list_word_keeps_the_remaining_space_and_history() {
    let source = "- one **bold** then *em*";
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), source, 2..5);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    frame(
        &mut app,
        &ctx,
        true,
        vec![key(Key::Backspace, egui::Modifiers::NONE)],
    );
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "•  bold then em"
    );
    app.undo_active();
    assert_eq!(app.session[0].content, source);
    app.redo_active();
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "•  bold then em"
    );
}

#[test]
fn hybrid_replacing_text_at_formatting_boundaries_keeps_visible_text() {
    for (source, selection, replacement, expected) in [
        ("*italic abc*", 1..2, " ", " talic abc"),
        (
            "prefix **strong** _em_ suffix",
            17..18,
            "新",
            "prefix strong新em suffix",
        ),
        ("a $x+y$ tail", 0..4, "新", "新+y tail"),
        ("a $x+y$ tail", 4..8, "新", "a x新tail"),
        ("a $x+y$ tail", 3..4, " ", "a  +y tail"),
        ("a <https://x.test> z", 3..4, "新", "a 新ttps://x.test z"),
        ("a <https://x.test> z", 8..11, " ", "a https x.test z"),
        (
            "a [label][] z\n\n[label]: https://x.test",
            3..4,
            "新",
            "a 新abel z",
        ),
        (
            "a [label] z\n\n[label]: https://x.test",
            3..4,
            "新",
            "a 新abel z",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), source, selection);
        let ctx = Context::default();
        install_fonts(&ctx);
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![egui::Event::Text(replacement.into())],
        );
        assert_eq!(
            VisualProjection::from_markdown(&app.session[0].content).text(),
            expected,
            "source after edit: {:?}",
            app.session[0].content
        );
        app.undo_active();
        assert_eq!(app.session[0].content, source);
        app.redo_active();
        assert_eq!(
            VisualProjection::from_markdown(&app.session[0].content).text(),
            expected
        );
    }
}

#[test]
fn hybrid_typing_before_trailing_reference_definitions_keeps_the_link_and_history() {
    let source = "a [label][ref] z\n\n[ref]: /x";
    let cursor = source[..source.find('\n').unwrap()].chars().count();
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), source, cursor..cursor);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    assert_eq!(VisualProjection::from_markdown(source).text(), "a label z");

    frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "a label zX"
    );
    assert!(app.session[0].content.contains("\n\n[ref]: /x"));
    app.undo_active();
    assert_eq!(app.session[0].content, source);
    app.redo_active();
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "a label zX"
    );
}

#[test]
fn hybrid_deleting_table_cell_text_keeps_the_table_and_history() {
    let source = "| A | **B** |\n| --- | --- |\n| x | y |";
    for (character, expected) in [
        ('A', "|  | **B** |\n| --- | --- |\n| x | y |"),
        ('x', "| A | **B** |\n| --- | --- |\n|  | y |"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let start = source.find(character).unwrap();
        let mut app = app_at(directory.path(), source, start..start + 1);
        let ctx = Context::default();
        install_fonts(&ctx);
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![key(Key::Backspace, egui::Modifiers::NONE)],
        );
        assert_eq!(app.session[0].content, expected);
        app.undo_active();
        assert_eq!(app.session[0].content, source);
        app.redo_active();
        assert_eq!(app.session[0].content, expected);
    }
}

#[test]
fn hybrid_typing_on_the_table_divider_keeps_the_table_and_history() {
    let source = "| left | right |\n| --- | --- |\n| cell | value |";
    let directory = tempfile::tempdir().unwrap();
    let divider = source.find("| ---").unwrap();
    let mut app = app_at(directory.path(), source, divider..divider);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    frame(&mut app, &ctx, true, vec![egui::Event::Text("中".into())]);

    let expected = "| left | right |\n| --- | --- |\n| 中cell | value |";
    assert_eq!(app.session[0].content, expected);
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "left  │  right\n\n中cell  │  value"
    );
    app.undo_active();
    assert_eq!(app.session[0].content, source);
    app.redo_active();
    assert_eq!(app.session[0].content, expected);
}

#[test]
fn hybrid_replacing_table_cell_text_with_space_keeps_it_visible() {
    let source = "| A | **B** |\n| --- | --- |\n| x | y |";
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), source, 2..3);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    frame(&mut app, &ctx, true, vec![egui::Event::Text(" ".into())]);
    assert_eq!(
        app.session[0].content,
        "| &#32; | **B** |\n| --- | --- |\n| x | y |"
    );
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "   │  B\n\nx  │  y"
    );
    app.undo_active();
    assert_eq!(app.session[0].content, source);
    app.redo_active();
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "   │  B\n\nx  │  y"
    );
}

#[test]
fn hybrid_deleting_table_cell_tail_keeps_the_surviving_space() {
    let source = "| a b | c d |\n| --- | --- |\n| e f | g h |";
    let directory = tempfile::tempdir().unwrap();
    let b = source.find('b').unwrap();
    let mut app = app_at(directory.path(), source, b..b + 1);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    frame(
        &mut app,
        &ctx,
        true,
        vec![key(Key::Backspace, egui::Modifiers::NONE)],
    );
    assert_eq!(
        app.session[0].content,
        "| a&#32; | c d |\n| --- | --- |\n| e f | g h |"
    );
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "a   │  c d\n\ne f  │  g h"
    );
    app.undo_active();
    assert_eq!(app.session[0].content, source);
    app.redo_active();
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "a   │  c d\n\ne f  │  g h"
    );
}

#[test]
fn hybrid_deleting_table_cell_tail_keeps_multiple_surviving_spaces() {
    let source = "| a  b | c |\n| - | - |\n| d | e |";
    let directory = tempfile::tempdir().unwrap();
    let b = source.find('b').unwrap();
    let mut app = app_at(directory.path(), source, b..b + 1);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    frame(
        &mut app,
        &ctx,
        true,
        vec![key(Key::Backspace, egui::Modifiers::NONE)],
    );
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "a    │  c\n\nd  │  e"
    );
    frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "a  X  │  c\n\nd  │  e"
    );
}

#[test]
fn hybrid_replacing_last_column_character_with_space_keeps_follow_up_input_in_cell() {
    let source = "| a | b |\n| - | - |\n| c | d |";
    let directory = tempfile::tempdir().unwrap();
    let b = source.find('b').unwrap();
    let mut app = app_at(directory.path(), source, b..b + 1);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    frame(&mut app, &ctx, true, vec![egui::Event::Text(" ".into())]);
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "a  │   \n\nc  │  d"
    );
    frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "a  │   X\n\nc  │  d"
    );
}

#[test]
fn hybrid_typing_after_deleting_quote_first_character_keeps_the_quote() {
    let source = "> a  b";
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), source, 2..3);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    frame(
        &mut app,
        &ctx,
        true,
        vec![key(Key::Backspace, egui::Modifiers::NONE)],
    );
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "│   b"
    );
    frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
    assert_eq!(app.session[0].content, "> X  b");
}

#[test]
fn hybrid_quote_tab_prefix_keeps_nested_list_marker_while_typing() {
    let source = ">\t- item";
    let directory = tempfile::tempdir().unwrap();
    let end = source.chars().count();
    let mut app = app_at(directory.path(), source, end..end);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "│ • item"
    );
    frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "│ • itemX"
    );
}

#[test]
fn hybrid_deleting_heading_first_character_keeps_surviving_spaces() {
    let source = "## a  b";
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), source, 3..4);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    frame(
        &mut app,
        &ctx,
        true,
        vec![key(Key::Backspace, egui::Modifiers::NONE)],
    );
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "  b"
    );
    frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "X  b"
    );
}

#[test]
fn hybrid_deleting_a_header_cell_keeps_the_caret_in_that_cell() {
    let source = "| a | b |\n| - | - |\n| c | d |";
    let directory = tempfile::tempdir().unwrap();
    let b = source.find('b').unwrap();
    let mut app = app_at(directory.path(), source, b..b + 1);
    let ctx = Context::default();
    install_fonts(&ctx);
    frame(&mut app, &ctx, true, vec![]);
    frame(
        &mut app,
        &ctx,
        true,
        vec![key(Key::Backspace, egui::Modifiers::NONE)],
    );
    assert_eq!(app.session[0].content, "| a |  |\n| - | - |\n| c | d |");
    let active = VisualProjection::from_markdown_with_selection(
        &app.session[0].content,
        Some(app.active_selection(0)),
    );
    assert_eq!(
        active.visual_char_range(&app.session[0].content, app.active_selection(0)),
        6..6
    );

    frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
    assert_eq!(
        VisualProjection::from_markdown(&app.session[0].content).text(),
        "a  │  X\n\nc  │  d"
    );
}

#[test]
fn hybrid_follow_up_typing_keeps_surviving_spaces_and_table_padding() {
    for (source, selected, replacement, expected) in [
        ("a b", 0..1, " ", " X b"),
        ("| a | b |\n| - | - |", 2..3, "新", "| 新X | b |\n| - | - |"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), source, selected);
        let ctx = Context::default();
        install_fonts(&ctx);
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![egui::Event::Text(replacement.into())],
        );
        frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
        assert_eq!(app.session[0].content, expected, "source={source:?}");
    }
}

#[test]
fn hybrid_typing_space_at_table_cell_end_keeps_it_visible() {
    let source = "| a | b |\n| - | - |\n| c | d |";
    for count in 1..=3 {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), source, 3..3);
        let ctx = Context::default();
        install_fonts(&ctx);
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![egui::Event::Text(" ".repeat(count))],
        );
        assert_eq!(
            VisualProjection::from_markdown(&app.session[0].content).text(),
            format!("a{}  │  b\n\nc  │  d", " ".repeat(count))
        );

        frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
        assert_eq!(
            VisualProjection::from_markdown(&app.session[0].content).text(),
            format!("a{}X  │  b\n\nc  │  d", " ".repeat(count))
        );
    }
}

#[test]
fn hybrid_typing_inside_an_indented_later_paragraph_keeps_its_separator() {
    for (source, cursor, expected) in [
        ("第一段\n\n  第二段", 5, "第一段\n\nX  第二段"),
        ("第一段\n\n  第二段", 6, "第一段\n\n X 第二段"),
        ("第一段\r\n\r\n  第二段", 7, "第一段\r\n\r\nX  第二段"),
        ("第一段\r\n\r\n  第二段", 8, "第一段\r\n\r\n X 第二段"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), source, cursor..cursor);
        let ctx = Context::default();
        install_fonts(&ctx);
        frame(&mut app, &ctx, true, vec![]);
        frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
        assert_eq!(app.session[0].content, expected, "cursor={cursor}");
    }
}

#[test]
fn leading_edits_before_enter_keep_their_input_semantics_and_history() {
    let cases = [
        (
            "中文🙂",
            3..3,
            key(Key::Delete, egui::Modifiers::NONE),
            "中文🙂\n\n新",
        ),
        (
            "中文🙂",
            0..0,
            key(Key::Backspace, egui::Modifiers::NONE),
            "\n\n新中文🙂",
        ),
        (
            "中文🙂",
            3..3,
            egui::Event::Paste(String::new()),
            "中文🙂\n\n新",
        ),
        (
            "```\n中文\n```",
            4..6,
            egui::Event::Paste("https://example.com".into()),
            "```\nhttps://example.com\n新\n```",
        ),
        (
            "中文🙂",
            3..3,
            egui::Event::Paste("(".into()),
            "中文🙂(\n\n新",
        ),
        (
            "中文🙂",
            3..3,
            egui::Event::Text("(".into()),
            "中文🙂(\n\n新)",
        ),
        ("()", 1..1, egui::Event::Text(")".into()), "()\n\n新"),
        (
            "中文🙂后文",
            0..3,
            egui::Event::Paste("https://example.com".into()),
            "[中文🙂](https://example.com)\n\n新后文",
        ),
        (
            "- 中文🙂后文",
            5..5,
            key(Key::Backspace, egui::Modifiers::NONE),
            "- 中文\n- 新后文",
        ),
        (
            "- 中文🙂后文",
            5..5,
            key(Key::Delete, egui::Modifiers::NONE),
            "- 中文🙂\n- 新文",
        ),
        (
            "- 中文🙂后文",
            2..5,
            key(Key::Backspace, egui::Modifiers::NONE),
            "\n新后文",
        ),
    ];
    let mut failures = Vec::new();
    for (source, selection, first, expected) in cases {
        for batched in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), source, selection.clone());
            let ctx = Context::default();
            frame(&mut app, &ctx, true, vec![]);
            let events = vec![
                first.clone(),
                key(Key::Enter, egui::Modifiers::NONE),
                egui::Event::Text("新".into()),
            ];
            if batched {
                frame(&mut app, &ctx, true, events);
            } else {
                for event in events {
                    frame(&mut app, &ctx, true, vec![event]);
                }
            }
            if app.session[0].content != expected {
                failures.push(format!("source={source:?} selection={selection:?} first={first:?} batched={batched} expected={expected:?} actual={:?}", app.session[0].content));
            }
            let edited = app.session[0].content.clone();
            let cursor = app.active_selection(0);
            let mut undos = 0;
            while app.session[0].can_undo() {
                app.undo_active();
                undos += 1;
                assert!(undos < 8);
            }
            assert_eq!(app.session[0].content, source);
            for _ in 0..undos {
                app.redo_active();
            }
            assert_eq!(app.session[0].content, edited);
            assert_eq!(app.active_selection(0), cursor);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn navigation_then_enter_in_one_frame_matches_separate_frames() {
    let mut failures = Vec::new();
    for (source, cursor) in [
        ("- 中文🙂后文", 5),
        ("正文🙂后文", 4),
        ("1. 中文🙂后文", 6),
        ("`中文🙂`", 4),
        ("`中文🙂`", 5),
    ] {
        for navigation in [Key::ArrowLeft, Key::ArrowRight, Key::Home] {
            let events = vec![
                key(navigation, egui::Modifiers::NONE),
                key(Key::Enter, egui::Modifiers::NONE),
                egui::Event::Text("新".into()),
            ];
            let directory = tempfile::tempdir().unwrap();
            let mut separate = app_at(directory.path(), source, cursor..cursor);
            let separate_ctx = Context::default();
            frame(&mut separate, &separate_ctx, true, vec![]);
            for event in &events {
                frame(&mut separate, &separate_ctx, true, vec![event.clone()]);
            }
            let mut batched = app_at(directory.path(), source, cursor..cursor);
            let batched_ctx = Context::default();
            frame(&mut batched, &batched_ctx, true, vec![]);
            frame(&mut batched, &batched_ctx, true, events);
            if separate.session[0].content != batched.session[0].content {
                failures.push(format!(
                    "source={source:?} key={navigation:?} separate={:?} batched={:?}",
                    separate.session[0].content, batched.session[0].content
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn code_block_arrow_navigation_retains_new_editor_focus() {
    for body in ["中文🙂后文", "first row\n中文🙂后文\nlast row"] {
        for selection_length in [0, 2] {
            let source = format!("```rust\n{body}\n```");
            let at = source[..source.find("中文").unwrap()].chars().count() + 2;
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), &source, at..at + selection_length);
            let ctx = Context::default();
            frame(&mut app, &ctx, true, vec![]);
            let editor = app.editor_surface.widget_id().unwrap();
            frame(
                &mut app,
                &ctx,
                true,
                vec![key(Key::ArrowUp, egui::Modifiers::NONE)],
            );
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                Some(editor),
                "body={body:?} selection_length={selection_length}"
            );
            frame(&mut app, &ctx, true, vec![egui::Event::Text("新".into())]);
            assert!(app.session[0].content.contains('新'));
            app.undo_active();
            assert_eq!(app.session[0].content, source);
        }
    }
}

#[test]
fn navigation_then_enter_preserves_selection_direction_and_history() {
    let shift = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    for source in [
        "- 中文🙂后文 e\u{301}尾".to_owned(),
        "> 中文🙂后文 e\u{301}尾".to_owned(),
        "```rust\nfirst row\n中文🙂后文 e\u{301}尾\nlast row\n```".to_owned(),
        format!("{}中文🙂后文", "wrapped words ".repeat(18)),
    ] {
        let at = source[..source.find("中文").unwrap()].chars().count() + 2;
        for reverse in [false, true] {
            for navigation in [
                vec![key(Key::ArrowLeft, shift), key(Key::ArrowLeft, shift)],
                vec![key(Key::ArrowRight, shift), key(Key::ArrowLeft, shift)],
                vec![key(Key::Home, shift)],
                vec![key(Key::End, egui::Modifiers::NONE)],
                vec![key(Key::ArrowUp, egui::Modifiers::NONE)],
                vec![key(Key::ArrowDown, egui::Modifiers::NONE)],
            ] {
                let mut events = navigation.clone();
                events.push(key(Key::Enter, egui::Modifiers::NONE));
                events.push(egui::Event::Text("新🙂".into()));
                let directory = tempfile::tempdir().unwrap();
                let mut outcomes = Vec::new();
                for batched in [false, true] {
                    let mut app = app_at(directory.path(), &source, at..at + 2);
                    if reverse {
                        app.editor_surface.select_cursor(egui::text::CCursorRange {
                            primary: CCursor::new(at),
                            secondary: CCursor::new(at + 2),
                            h_pos: None,
                        });
                    }
                    let ctx = Context::default();
                    frame(&mut app, &ctx, true, vec![]);
                    if batched {
                        frame(&mut app, &ctx, true, events.clone());
                    } else {
                        for event in &events {
                            frame(&mut app, &ctx, true, vec![event.clone()]);
                        }
                    }
                    let edited = app.session[0].content.clone();
                    let selection = app.active_selection(0);
                    assert!(
                        edited.contains("新🙂"),
                        "source={source:?} reverse={reverse} navigation={navigation:?} batched={batched} actual={edited:?}"
                    );
                    let mut undo_count = 0;
                    while app.session[0].can_undo() {
                        app.undo_active();
                        undo_count += 1;
                        assert!(undo_count <= 8);
                    }
                    assert_eq!(app.session[0].content, source);
                    for _ in 0..undo_count {
                        app.redo_active();
                    }
                    assert_eq!(app.session[0].content, edited);
                    assert_eq!(app.active_selection(0), selection);
                    outcomes.push((edited, selection));
                }
                assert_eq!(
                    outcomes[0], outcomes[1],
                    "source={source:?} reverse={reverse} navigation={navigation:?}"
                );
            }
        }
    }
}

#[test]
fn setext_heading_enter_keeps_heading_level_and_history() {
    for underline in ["---", "===", "   ----  "] {
        for (body, cursor, expected_body) in [
            ("中文🙂", 3, "中文🙂"),
            ("中文🙂后文", 3, "中文🙂"),
            ("**中文**", 3, "**中**"),
            ("甲\n中文🙂", 5, "甲\n中文🙂"),
        ] {
            for batched in [false, true] {
                let source = format!("{body}\n{underline}\n\n后段");
                let directory = tempfile::tempdir().unwrap();
                let mut app = app_at(directory.path(), &source, cursor..cursor);
                let ctx = Context::default();
                install_fonts(&ctx);
                frame(&mut app, &ctx, true, vec![]);
                let events = vec![
                    key(Key::Enter, egui::Modifiers::NONE),
                    egui::Event::Text("新".into()),
                ];
                if batched {
                    frame(&mut app, &ctx, true, events);
                } else {
                    for event in events {
                        frame(&mut app, &ctx, true, vec![event]);
                    }
                }
                let html = markdown::render_html_fragment(&app.session[0].content);
                let heading_html =
                    markdown::render_html_fragment(&format!("{expected_body}\n{underline}"));
                assert!(
                    html.starts_with(&heading_html),
                    "source={source:?}, actual={:?}, html={html:?}",
                    app.session[0].content
                );
                assert!(html.contains("新"));
                assert!(html.ends_with("<p>后段</p>\n"));
                assert!(!html.contains("<hr>"));
                let before_cursor =
                    char_to_byte(&app.session[0].content, app.active_selection(0).start);
                assert!(app.session[0].content[..before_cursor].ends_with('新'));
                while app.session[0].can_undo() {
                    app.undo_active();
                }
                assert_eq!(app.session[0].content, source);
                while app.session[0].can_redo() {
                    app.redo_active();
                }
                assert!(
                    markdown::render_html_fragment(&app.session[0].content)
                        .starts_with(&heading_html)
                );
            }
        }
    }
}

#[test]
fn indented_code_enter_preserves_literal_text_and_undo() {
    for indent in ["    ", "\t", "      "] {
        for shift in [false, true] {
            for batched in [false, true] {
                let source = format!("前文\n\n{indent}中文🙂XYZ\n{indent}**literal**\n\n后文");
                let at = 4 + indent.chars().count() + 2;
                let directory = tempfile::tempdir().unwrap();
                let mut app = app_at(directory.path(), &source, at..at);
                let ctx = Context::default();
                install_fonts(&ctx);
                frame(&mut app, &ctx, true, vec![]);
                let events = vec![
                    key(
                        Key::Enter,
                        egui::Modifiers {
                            shift,
                            ..Default::default()
                        },
                    ),
                    egui::Event::Text("新".into()),
                ];
                if batched {
                    frame(&mut app, &ctx, true, events);
                } else {
                    for event in events {
                        frame(&mut app, &ctx, true, vec![event]);
                    }
                }
                let expected =
                    format!("前文\n\n{indent}中文\n{indent}新🙂XYZ\n{indent}**literal**\n\n后文");
                assert_eq!(
                    app.session[0].content, expected,
                    "indent={indent:?}, shift={shift}, batched={batched}"
                );
                let caret = at + 2 + indent.chars().count();
                assert_eq!(app.active_selection(0), caret..caret);
                assert_eq!(
                    markdown::render_html_fragment(&expected)
                        .matches("<pre><code>")
                        .count(),
                    1
                );
                while app.session[0].can_undo() {
                    app.undo_active();
                }
                assert_eq!(app.session[0].content, source);
                while app.session[0].can_redo() {
                    app.redo_active();
                }
                assert_eq!(app.session[0].content, expected);
            }
        }
    }
}

#[test]
fn document_edge_navigation_types_inside_fenced_code() {
    for fence in ["```rust", "~~~text"] {
        let closer = &fence[..3];
        for nav in [Key::Home, Key::End] {
            for batched in [false, true] {
                for body in ["中文🙂", ""] {
                    let source = format!("{fence}\n{body}\n{closer}");
                    let directory = tempfile::tempdir().unwrap();
                    let mut app = app_at(directory.path(), &source, 0..0);
                    let ctx = Context::default();
                    install_fonts(&ctx);
                    frame(&mut app, &ctx, true, vec![]);
                    let events = vec![key(nav, command()), egui::Event::Text("新".into())];
                    if batched {
                        frame(&mut app, &ctx, true, events);
                    } else {
                        for event in events {
                            frame(&mut app, &ctx, true, vec![event]);
                        }
                    }
                    let expected = if nav == Key::Home {
                        format!("{fence}\n新{body}\n{closer}")
                    } else {
                        format!("{fence}\n{body}新\n{closer}")
                    };
                    assert_eq!(
                        app.session[0].content, expected,
                        "nav={nav:?}, batched={batched}"
                    );
                    let expected_cursor = fence.chars().count()
                        + 2
                        + if nav == Key::End {
                            body.chars().count()
                        } else {
                            0
                        };
                    assert_eq!(app.active_selection(0), expected_cursor..expected_cursor);
                    app.undo_active();
                    assert_eq!(app.session[0].content, source);
                    app.redo_active();
                    assert_eq!(app.session[0].content, expected);
                }
            }
        }
    }
}

#[test]
fn partial_inline_delete_and_batched_backspace_keep_markup_and_history() {
    for (source, selection, events, expected) in [
        (
            "**中文🙂** XYZ",
            4..8,
            vec![key(Key::Backspace, egui::Modifiers::NONE)],
            "**中文**XYZ",
        ),
        (
            "[中文🙂](foo.md)XYZ",
            14..14,
            vec![
                key(Key::Backspace, egui::Modifiers::NONE),
                key(Key::Backspace, egui::Modifiers::NONE),
            ],
            "[中文](foo.md)YZ",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), source, selection);
        let ctx = Context::default();
        install_fonts(&ctx);
        frame(&mut app, &ctx, true, vec![]);
        frame(&mut app, &ctx, true, events);
        assert_eq!(app.session[0].content, expected);
        app.undo_active();
        assert_eq!(app.session[0].content, source);
        app.redo_active();
        assert_eq!(app.session[0].content, expected);
    }
}

#[test]
fn deleting_across_emphasis_opener_keeps_rendering_cursor_and_history() {
    for delimiter in ["*", "**", "_", "__", "~~"] {
        let source = format!("甲 {delimiter}乙 丙🙂{delimiter}");
        let projection = crate::wysiwyg::VisualProjection::from_markdown(&source);
        let selection = projection.source_char_range(&source, 1..3);
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), &source, selection);
        let ctx = Context::default();
        install_fonts(&ctx);
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![key(Key::Backspace, egui::Modifiers::NONE)],
        );
        let deleted = app.session[0].content.to_string();
        let active = crate::wysiwyg::VisualProjection::from_markdown_with_selection(
            &deleted,
            Some(app.active_selection(0)),
        );
        assert_eq!(
            active.text(),
            "甲 丙🙂",
            "source={source:?}, deleted={deleted:?}"
        );
        assert_eq!(
            active.visual_char_range(&deleted, app.active_selection(0)),
            1..1
        );
        app.undo_active();
        assert_eq!(app.session[0].content, source);
        app.redo_active();
        assert_eq!(app.session[0].content, deleted);
        frame(&mut app, &ctx, true, vec![egui::Event::Text("新".into())]);
        let after = &app.session[0].content;
        let active = crate::wysiwyg::VisualProjection::from_markdown_with_selection(
            after,
            Some(app.active_selection(0)),
        );
        assert_eq!(active.text(), "甲新 丙🙂");
        assert_eq!(
            active.visual_char_range(after, app.active_selection(0)),
            2..2
        );
    }
}

#[test]
fn ime_cancel_preserves_following_input_and_history() {
    for empty_preedit in [false, true] {
        for batched in [false, true] {
            for selection in [1..1, 1..3] {
                let directory = tempfile::tempdir().unwrap();
                let mut app = app_at(directory.path(), "A中文B", selection.clone());
                let ctx = Context::default();
                frame(&mut app, &ctx, true, vec![]);
                frame(
                    &mut app,
                    &ctx,
                    true,
                    vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: "ni".into(),
                        active_range_chars: Some(0..2),
                    })],
                );
                assert_eq!(app.session[0].content, "A中文B");
                let cancel = if empty_preedit {
                    egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: String::new(),
                        active_range_chars: None,
                    })
                } else {
                    egui::Event::Ime(egui::ImeEvent::Commit(String::new()))
                };
                let events = vec![
                    cancel,
                    egui::Event::Text("🙂".into()),
                    key(Key::Enter, egui::Modifiers::NONE),
                    egui::Event::Text("新".into()),
                ];
                if batched {
                    frame(&mut app, &ctx, true, events);
                } else {
                    for event in events {
                        frame(&mut app, &ctx, true, vec![event]);
                    }
                }
                let expected = "A🙂\n\n新中文B";
                assert_eq!(
                    app.session[0].content, expected,
                    "batched={batched}, selection={selection:?}"
                );
                assert_eq!(app.active_selection(0), 5..5);
                while app.session[0].can_undo() {
                    app.undo_active();
                }
                assert_eq!(app.session[0].content, "A中文B");
                while app.session[0].can_redo() {
                    app.redo_active();
                }
                assert_eq!(app.session[0].content, expected);
            }
        }
    }
}

#[test]
fn full_audit_chaotic_input_history_restores_unicode_and_markdown_exactly() {
    for original in [
        "FIRST 中文🙂\n\nTAIL",
        "FIRST\n\n```text\n****\nA`B\n```\n\nTAIL",
        "# Heading\n\n**bold** and `` ` ``\n\n> quote\n\n- [ ] task",
        "| A | B |\n| --- | --- |\n| 中文 | 🙂 |\n\nTAIL",
        "~~~\ncode\n~~~\n\n\n\nlast",
        "- one\n  - nested\n\n[link](https://example.com)\n\n$$x$$",
    ] {
        for seed in [7_u64, 29, 113] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), original, 0..0);
            let ctx = Context::default();
            let mut fonts = FontDefinitions::default();
            fonts.families.insert(
                FontFamily::Name(WYSIWYG_STRONG_FAMILY.into()),
                fonts.families[&FontFamily::Proportional].clone(),
            );
            ctx.set_fonts(fonts);
            let mut hybrid = true;
            frame(&mut app, &ctx, hybrid, vec![]);
            let mut state = seed;
            for step in 0..64 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let event = match state % 16 {
                    0 => key(Key::Home, command()),
                    1 => key(Key::End, command()),
                    2 => key(Key::ArrowLeft, egui::Modifiers::NONE),
                    3 => key(Key::ArrowRight, egui::Modifiers::SHIFT),
                    4 => key(Key::ArrowUp, egui::Modifiers::NONE),
                    5 => key(Key::ArrowDown, egui::Modifiers::SHIFT),
                    6 => key(Key::Backspace, egui::Modifiers::NONE),
                    7 => key(Key::Delete, egui::Modifiers::NONE),
                    8 => key(Key::Enter, egui::Modifiers::NONE),
                    9 => key(Key::Enter, egui::Modifiers::SHIFT),
                    10 => egui::Event::Text("中🙂".into()),
                    11 => egui::Event::Paste("α\r\nβ`*".into()),
                    12 => key(Key::Tab, egui::Modifiers::NONE),
                    13 => egui::Event::Text("(".into()),
                    14 => key(Key::Z, command()),
                    _ => {
                        hybrid = !hybrid;
                        app.execute(AppCommand::SetView(if hybrid {
                            ViewMode::Hybrid
                        } else {
                            ViewMode::Edit
                        }));
                        key(Key::End, egui::Modifiers::NONE)
                    }
                };
                frame(&mut app, &ctx, hybrid, vec![event]);
                let content = &app.session[0].content;
                assert!(!content.contains('\r'), "seed={seed} step={step}");
                if let Some(cursor) = app.editor_surface.cursor() {
                    assert!(
                        cursor.primary.index.0 <= content.chars().count(),
                        "seed={seed} step={step}: {content:?}"
                    );
                    assert!(
                        cursor.secondary.index.0 <= content.chars().count(),
                        "seed={seed} step={step}: {content:?}"
                    );
                }
            }
            let edited = app.session[0].content.clone();
            let mut undo_count = 0;
            while app.session[0].can_undo() {
                app.undo_active();
                undo_count += 1;
                assert!(undo_count <= 128);
            }
            assert_eq!(app.session[0].content, original, "seed={seed}");
            // Replay only the operations just undone: older redo branches can
            // legitimately remain after the last simulated key was Undo.
            for _ in 0..undo_count {
                app.redo_active();
            }
            assert_eq!(app.session[0].content, edited, "seed={seed}");
        }
    }
}

#[test]
fn full_audit_url_paste_inside_code_is_literal_in_both_editors() {
    for hybrid in [false, true] {
        for (source, selection) in [
            ("```text\nOLD\n```\n\nTAIL", 8..11),
            ("`OLD`\n\nTAIL", 1..4),
            ("    OLD\n\nTAIL", 4..7),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), source, selection);
            let ctx = Context::default();
            frame(&mut app, &ctx, hybrid, vec![]);
            frame(
                &mut app,
                &ctx,
                hybrid,
                vec![egui::Event::Paste("https://example.com/a".into())],
            );
            assert_eq!(
                app.session[0].content,
                source.replace("OLD", "https://example.com/a"),
                "hybrid={hybrid}"
            );
            frame(&mut app, &ctx, hybrid, vec![key(Key::Z, command())]);
            assert_eq!(app.session[0].content, source);
        }
    }
}

#[test]
fn full_audit_deleting_literal_stars_does_not_merge_code_lines() {
    let source = "FIRST\n\n```text\n****\nA`B\n```\n\nTAIL";
    let at = source.find("****").unwrap() + 2;
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), source, at..at);
    let ctx = Context::default();
    frame(&mut app, &ctx, true, vec![]);
    frame(
        &mut app,
        &ctx,
        true,
        vec![key(Key::Delete, egui::Modifiers::NONE)],
    );
    assert_eq!(app.session[0].content, source.replacen("****", "***", 1));
    frame(&mut app, &ctx, true, vec![key(Key::Z, command())]);
    assert_eq!(app.session[0].content, source);
}

#[test]
fn product_input_switch_to_source_keeps_target_line_visible() {
    let source = format!(
        "{}\n\nFINAL_TARGET",
        (0..90)
            .map(|n| format!("Paragraph {n}."))
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), &source, 0..0);
    let ctx = Context::default();
    app.state.view_mode = ViewMode::Edit;
    frame(&mut app, &ctx, false, vec![]);
    app.execute(AppCommand::SetView(ViewMode::Hybrid));
    app.queue_editor_selection(source.chars().count()..source.chars().count());
    frame(&mut app, &ctx, true, vec![]);
    app.execute(AppCommand::SetView(ViewMode::Edit));
    let mut visible = false;
    for step in 0..5 {
        let output = ctx.run_ui(
            egui::RawInput {
                time: Some(1.0 + step as f64),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                app.edit_pane(ui, 0, None);
            },
        );
        fn target_visible(shape: &egui::Shape, clip: egui::Rect) -> bool {
            match shape {
                egui::Shape::Text(text) => {
                    let Some(byte) = text.galley.job.text.find("FINAL_TARGET") else {
                        return false;
                    };
                    let start = text.galley.job.text[..byte].chars().count();
                    let target = text
                        .galley
                        .pos_from_cursor(CCursor::new(start))
                        .union(
                            text.galley
                                .pos_from_cursor(CCursor::new(start + "FINAL_TARGET".len())),
                        )
                        .translate(text.pos.to_vec2());
                    clip.contains_rect(target)
                }
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| target_visible(shape, clip)),
                _ => false,
            }
        }
        visible |= output
            .shapes
            .iter()
            .any(|shape| target_visible(&shape.shape, shape.clip_rect));
    }
    assert_eq!(app.session[0].content, source);
    assert!(
        visible,
        "switching to source must reveal the actual target line, not merely its full-document galley"
    );
}

#[test]
fn product_input_second_select_all_expands_to_document() {
    for source in [
        "first\n\nmiddle\n\nlast",
        "first\n\n```rust\ncode\n```\n\nlast",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let cursor = source
            .find(if source.contains("```") {
                "code"
            } else {
                "middle"
            })
            .unwrap();
        let mut app = app_at(directory.path(), source, cursor..cursor);
        let ctx = Context::default();
        frame(&mut app, &ctx, true, vec![]);
        frame(&mut app, &ctx, true, vec![key(Key::A, command())]);
        frame(&mut app, &ctx, true, vec![key(Key::A, command())]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![egui::Event::Text("REPLACED".into())],
        );
        assert_eq!(app.session[0].content, "REPLACED");
    }
}

#[test]
fn product_input_page_navigation_crosses_paragraphs() {
    let source = (0..70)
        .map(|n| format!("Paragraph {n}."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), &source, 0..0);
    let ctx = Context::default();
    frame(&mut app, &ctx, true, vec![]);
    frame(
        &mut app,
        &ctx,
        true,
        vec![key(Key::PageDown, egui::Modifiers::NONE)],
    );
    let after_down = app.editor_surface.cursor().unwrap().primary.index.0;
    assert!(after_down > source.find("Paragraph 3.").unwrap());
    frame(
        &mut app,
        &ctx,
        true,
        vec![key(Key::PageUp, egui::Modifiers::NONE)],
    );
    assert!(app.editor_surface.cursor().unwrap().primary.index.0 < after_down);
    assert_eq!(app.session[0].content, source);
}

fn app_at(path: &Path, source: &str, selection: std::ops::Range<usize>) -> RuporaApp {
    let mut app = RuporaApp::from_state(
        PersistedState::default(),
        RecoveryStore::at(path.join("recovery.json")),
        None,
        ExtensionRegistry::disabled(path.join("extensions.json")),
        None,
    );
    app.new_document();
    app.session[0].content = source.to_owned();
    app.session[0].update_after_edit();
    app.queue_editor_selection(selection);
    app
}

fn key(key: Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn command() -> egui::Modifiers {
    egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    }
}

fn frame(app: &mut RuporaApp, ctx: &Context, hybrid: bool, events: Vec<egui::Event>) {
    let modifiers = events
        .iter()
        .find_map(|event| match event {
            egui::Event::Key { modifiers, .. } => Some(*modifiers),
            _ => None,
        })
        .unwrap_or_default();
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 800.0),
        )),
        events,
        modifiers,
        ..Default::default()
    };
    eframe::App::raw_input_hook(app, ctx, &mut raw);
    let _ = ctx.run_ui(raw, |ui| {
        app.handle_shortcuts(ui.ctx());
        if hybrid {
            app.hybrid_pane(ui, 0);
        } else {
            app.edit_pane(ui, 0, None);
        }
    });
}

#[test]
fn product_input_select_all_same_frame() {
    let mut failures = Vec::new();
    for hybrid in [false, true] {
        for replacement in ["替换", "(", "["] {
            let source = "正文🙂\n\n- [ ] KEEP";
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), source, 1..1);
            let ctx = Context::default();
            frame(&mut app, &ctx, hybrid, vec![]);
            frame(
                &mut app,
                &ctx,
                hybrid,
                vec![
                    key(Key::A, command()),
                    egui::Event::Text(replacement.into()),
                ],
            );
            let sequential_directory = tempfile::tempdir().unwrap();
            let mut sequential = app_at(sequential_directory.path(), source, 1..1);
            let sequential_ctx = Context::default();
            frame(&mut sequential, &sequential_ctx, hybrid, vec![]);
            frame(
                &mut sequential,
                &sequential_ctx,
                hybrid,
                vec![key(Key::A, command())],
            );
            frame(
                &mut sequential,
                &sequential_ctx,
                hybrid,
                vec![egui::Event::Text(replacement.into())],
            );
            let expected = sequential.session[0].content.clone();
            if app.session[0].content != expected {
                failures.push(format!(
                    "hybrid={hybrid} replacement={replacement:?} expected={expected:?} actual={:?}",
                    app.session[0].content
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_cross_block_navigation_before_text() {
    let mut failures = Vec::new();
    for nav in [Key::ArrowLeft, Key::ArrowRight, Key::Home, Key::End] {
        let mut results = Vec::new();
        for batched in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), "one\n\ntwo", 1..7);
            let ctx = Context::default();
            frame(&mut app, &ctx, true, vec![]);
            let events = vec![
                key(nav, egui::Modifiers::NONE),
                egui::Event::Text("X".into()),
            ];
            if batched {
                frame(&mut app, &ctx, true, events);
            } else {
                for event in events {
                    frame(&mut app, &ctx, true, vec![event]);
                }
            }
            results.push(app.session[0].content.clone());
        }
        let expected = match nav {
            Key::ArrowLeft => Some("oXne\n\ntwo"),
            Key::ArrowRight => Some("one\n\ntwXo"),
            _ => None,
        };
        if results[0] != results[1]
            || expected.is_some_and(|expected| results[0] != expected || results[1] != expected)
        {
            failures.push(format!(
                "nav={nav:?} expected={expected:?} sequential={:?} batched={:?}",
                results[0], results[1]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_selection_before_unselected_third_block_does_not_panic() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), "one\n\ntwo\n\nthree", 1..7);
    frame(&mut app, &Context::default(), true, vec![]);
    assert_eq!(app.session[0].content, "one\n\ntwo\n\nthree");
}

#[test]
fn product_input_cross_block_deletion_undo_redo() {
    for event in [
        key(Key::Delete, egui::Modifiers::NONE),
        key(Key::Backspace, egui::Modifiers::NONE),
        egui::Event::Cut,
    ] {
        let source = "中文🙂\n\n```rust\ncode\n```\n\n尾部";
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), source, 1..24);
        let ctx = Context::default();
        frame(&mut app, &ctx, true, vec![]);
        frame(&mut app, &ctx, true, vec![event]);
        let edited = app.session[0].content.clone();
        frame(&mut app, &ctx, true, vec![key(Key::Z, command())]);
        assert_eq!(app.session[0].content, source);
        frame(&mut app, &ctx, true, vec![key(Key::Y, command())]);
        assert_eq!(app.session[0].content, edited);
    }
}

#[test]
fn product_input_ime_commit_then_text() {
    for hybrid in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), "AB", 1..1);
        let ctx = Context::default();
        frame(&mut app, &ctx, hybrid, vec![]);
        frame(
            &mut app,
            &ctx,
            hybrid,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "ni".into(),
                active_range_chars: Some(0..2),
            })],
        );
        frame(
            &mut app,
            &ctx,
            hybrid,
            vec![
                egui::Event::Ime(egui::ImeEvent::Commit("你".into())),
                egui::Event::Text("🙂".into()),
            ],
        );
        assert_eq!(app.session[0].content, "A你🙂B", "hybrid={hybrid}");
    }
}

#[test]
fn product_input_ime_committed_text_survives_next_composition_cancel() {
    let mut failures = Vec::new();
    for hybrid in [false, true] {
        for batched in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), "AB", 1..1);
            let ctx = Context::default();
            frame(&mut app, &ctx, hybrid, vec![]);
            let preedit = |text: &str| {
                egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: text.into(),
                    active_range_chars: Some(0..text.chars().count()),
                })
            };
            frame(&mut app, &ctx, hybrid, vec![preedit("ni")]);
            let events = vec![
                egui::Event::Ime(egui::ImeEvent::Commit("你".into())),
                preedit("hao"),
            ];
            if batched {
                frame(&mut app, &ctx, hybrid, events);
            } else {
                for event in events {
                    frame(&mut app, &ctx, hybrid, vec![event]);
                }
            }
            frame(
                &mut app,
                &ctx,
                hybrid,
                vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new()))],
            );
            if app.session[0].content != "A你B" {
                failures.push(format!(
                    "hybrid={hybrid} batched={batched} actual={:?}",
                    app.session[0].content
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_mode_switch_after_unicode_edit_preserves_undo_redo() {
    let source = "前文🙂\n\n```rust\ncode\n```\n\n尾部";
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), source, 1..2);
    let ctx = Context::default();
    frame(&mut app, &ctx, true, vec![]);
    frame(&mut app, &ctx, true, vec![egui::Event::Text("变更".into())]);
    let edited = app.session[0].content.clone();
    app.execute(AppCommand::SetView(ViewMode::Edit));
    frame(&mut app, &ctx, false, vec![]);
    assert_eq!(app.session[0].content, edited);
    frame(&mut app, &ctx, false, vec![key(Key::Z, command())]);
    assert_eq!(app.session[0].content, source);
    app.execute(AppCommand::SetView(ViewMode::Hybrid));
    frame(&mut app, &ctx, true, vec![]);
    frame(&mut app, &ctx, true, vec![key(Key::Y, command())]);
    assert_eq!(app.session[0].content, edited);
}

#[test]
fn product_input_control_home_end_navigate_whole_document() {
    let mut failures = Vec::new();
    for nav in [Key::Home, Key::End] {
        for shift in [false, true] {
            let source = "first\n\nmiddle\n\nlast";
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), source, 9..9);
            let ctx = Context::default();
            frame(&mut app, &ctx, true, vec![]);
            let modifiers = egui::Modifiers { shift, ..command() };
            frame(&mut app, &ctx, true, vec![key(nav, modifiers)]);
            frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
            let expected = match (nav, shift) {
                (Key::Home, false) => "Xfirst\n\nmiddle\n\nlast",
                (Key::End, false) => "first\n\nmiddle\n\nlastX",
                (Key::Home, true) => "Xddle\n\nlast",
                (Key::End, true) => "first\n\nmiX",
                _ => unreachable!(),
            };
            if app.session[0].content != expected {
                failures.push(format!(
                    "nav={nav:?} shift={shift} expected={expected:?} actual={:?}",
                    app.session[0].content
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_fence_boundary_multi_event_matches_sequential() {
    let mut failures = Vec::new();
    for (source, cursor, nav) in [
        ("前文\n```rust\ncode\n```\n\nTAIL", 2, Key::Delete),
        ("前文\n\n```rust\ncode\n```\n\nTAIL", 2, Key::Delete),
        ("前文\n\n```rust\ncode\n```\n\nTAIL", 12, Key::Backspace),
        ("```rust\ncode\n```\n\nTAIL", 12, Key::ArrowRight),
    ] {
        let mut results = Vec::new();
        for batched in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), source, cursor..cursor);
            let ctx = Context::default();
            frame(&mut app, &ctx, true, vec![]);
            let events = vec![
                key(nav, egui::Modifiers::NONE),
                egui::Event::Text("X".into()),
            ];
            if batched {
                frame(&mut app, &ctx, true, events);
            } else {
                for event in events {
                    frame(&mut app, &ctx, true, vec![event]);
                }
            }
            results.push(app.session[0].content.clone());
        }
        if results[0] != results[1] {
            failures.push(format!(
                "nav={nav:?} source={source:?} sequential={:?} batched={:?}",
                results[0], results[1]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_text_then_undo_in_same_frame_obeys_event_order() {
    let mut failures = Vec::new();
    for hybrid in [false, true] {
        let mut results = Vec::new();
        for batched in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), "AB", 2..2);
            let ctx = Context::default();
            frame(&mut app, &ctx, hybrid, vec![]);
            frame(&mut app, &ctx, hybrid, vec![egui::Event::Text("C".into())]);
            let events = vec![egui::Event::Text("X".into()), key(Key::Z, command())];
            if batched {
                frame(&mut app, &ctx, hybrid, events);
            } else {
                for event in events {
                    frame(&mut app, &ctx, hybrid, vec![event]);
                }
            }
            results.push(app.session[0].content.clone());
        }
        if results[0] != results[1] {
            failures.push(format!(
                "hybrid={hybrid} sequential={:?} batched={:?}",
                results[0], results[1]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_extreme_unicode_delete_undo_roundtrip() {
    for hybrid in [false, true] {
        for payload in ["🙂", "𠀀", "e\u{301}", "👨‍👩‍👧‍👦", "\t", "α\r\nβ\rγ"]
        {
            let directory = tempfile::tempdir().unwrap();
            let mut app = app_at(directory.path(), "AB", 1..1);
            let ctx = Context::default();
            frame(&mut app, &ctx, hybrid, vec![]);
            frame(
                &mut app,
                &ctx,
                hybrid,
                vec![egui::Event::Paste(payload.into())],
            );
            let inserted = app.session[0].content.clone();
            assert_eq!(
                inserted,
                format!("A{}B", crate::document::normalize_line_endings(payload)),
                "hybrid={hybrid} payload={payload:?}"
            );
            frame(
                &mut app,
                &ctx,
                hybrid,
                vec![key(Key::Backspace, egui::Modifiers::NONE)],
            );
            let deleted = app.session[0].content.clone();
            frame(&mut app, &ctx, hybrid, vec![key(Key::Z, command())]);
            // Typing coalescing may group the paste and deletion into one transaction.
            assert!(
                app.session[0].content == inserted || app.session[0].content == "AB",
                "hybrid={hybrid} payload={payload:?} unexpected history={:?}",
                app.session[0].content
            );
            frame(&mut app, &ctx, hybrid, vec![key(Key::Y, command())]);
            assert_eq!(app.session[0].content, deleted);
        }
    }
}

#[test]
fn product_input_delete_paragraph_after_fence_then_type_preserves_boundary() {
    for delete_first in [false, true] {
        let source = "```rust\ncode\n```\n\npara KEEP\n\nEND";
        let directory = tempfile::tempdir().unwrap();
        let cursor = "```rust\ncode\n```\n\npa".chars().count();
        let mut app = app_at(directory.path(), source, cursor..cursor);
        let ctx = Context::default();
        frame(&mut app, &ctx, true, vec![]);
        frame(&mut app, &ctx, true, vec![key(Key::A, command())]);
        if delete_first {
            frame(
                &mut app,
                &ctx,
                true,
                vec![key(Key::Delete, egui::Modifiers::NONE)],
            );
        }
        frame(
            &mut app,
            &ctx,
            true,
            vec![egui::Event::Text("REPLACE".into())],
        );
        assert_eq!(
            app.session[0].content, "```rust\ncode\n```\n\nREPLACE\n\nEND",
            "delete_first={delete_first}"
        );
    }
}

#[test]
fn product_input_switch_from_source_tail_to_hybrid_keeps_caret_visible() {
    let mut source = (0..90)
        .map(|i| format!("Paragraph {i} with ordinary content."))
        .collect::<Vec<_>>()
        .join("\n\n");
    source.push_str("\n\nFINAL_TARGET");
    let directory = tempfile::tempdir().unwrap();
    let end = source.chars().count();
    let mut app = app_at(directory.path(), &source, end..end);
    let ctx = Context::default();
    app.state.view_mode = ViewMode::Edit;
    for _ in 0..3 {
        frame(&mut app, &ctx, false, vec![]);
    }
    app.execute(AppCommand::SetView(ViewMode::Hybrid));
    let mut visible = false;
    for step in 0..5 {
        let output = ctx.run_ui(
            egui::RawInput {
                time: Some(1.0 + step as f64),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| app.hybrid_pane(ui, 0),
        );
        fn is_target_visible(shape: &egui::Shape, clip: egui::Rect) -> bool {
            match shape {
                egui::Shape::Text(text) => {
                    text.galley.job.text.contains("FINAL_TARGET")
                        && clip.intersects(text.galley.rect.translate(text.pos.to_vec2()))
                }
                egui::Shape::Vec(shapes) => {
                    shapes.iter().any(|shape| is_target_visible(shape, clip))
                }
                _ => false,
            }
        }
        visible |= output
            .shapes
            .iter()
            .any(|shape| is_target_visible(&shape.shape, shape.clip_rect));
    }
    assert_eq!(app.session[0].content, source);
    assert!(
        visible,
        "source caret at document tail must scroll its active hybrid block into the actual painting clip"
    );
}

#[test]
fn product_input_hidden_separator_delete_removes_only_one_extra_break() {
    let mut failures = Vec::new();
    for (cursor, action) in [(7, Key::Backspace), (5, Key::Delete)] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), "FIRST\n\n\nNEXT", cursor..cursor);
        let ctx = Context::default();
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![key(action, egui::Modifiers::NONE)],
        );
        if app.session[0].content != "FIRST\n\nNEXT" {
            failures.push(format!(
                "cursor={cursor} key={action:?} actual={:?}",
                app.session[0].content
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_hidden_separator_backspace_preserves_inline_closer() {
    let mut failures = Vec::new();
    for (body, expected) in [
        ("**bold**", "**bol**"),
        ("*italic*", "*itali*"),
        ("~~strike~~", "~~strik~~"),
        ("`code`", "`cod`"),
    ] {
        let source = format!("{body}\n\nNEXT");
        let directory = tempfile::tempdir().unwrap();
        let cursor = body.chars().count();
        let mut app = app_at(directory.path(), &source, cursor..cursor);
        let ctx = Context::default();
        let mut fonts = FontDefinitions::default();
        fonts.families.insert(
            FontFamily::Name(WYSIWYG_STRONG_FAMILY.into()),
            fonts.families[&FontFamily::Proportional].clone(),
        );
        ctx.set_fonts(fonts);
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![key(Key::Backspace, egui::Modifiers::NONE)],
        );
        if app.session[0].content != format!("{expected}\n\nNEXT") {
            failures.push(format!("body={body:?} actual={:?}", app.session[0].content));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_hidden_separator_delete_at_plain_paragraph_end() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), "FIRST\n\nNEXT", 5..5);
    let ctx = Context::default();
    frame(&mut app, &ctx, true, vec![]);
    frame(
        &mut app,
        &ctx,
        true,
        vec![key(Key::Delete, egui::Modifiers::NONE)],
    );
    assert_eq!(app.session[0].content, "FIRSTNEXT");
}

#[test]
fn product_input_hidden_separator_enter_then_type_keeps_all_text() {
    for body in ["FIRST", "FIRST  "] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("{body}\n\nNEXT");
        let cursor = body.chars().count();
        let mut app = app_at(directory.path(), &source, cursor..cursor);
        let ctx = Context::default();
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![key(Key::Enter, egui::Modifiers::NONE)],
        );
        frame(&mut app, &ctx, true, vec![egui::Event::Text("NEW".into())]);
        assert_eq!(
            app.session[0].content,
            format!(
                "{body}{}NEW\n\nNEXT",
                if body.ends_with("  ") { "\n" } else { "\n\n" }
            ),
            "body={body:?}"
        );
    }
}

#[test]
fn product_input_hidden_separator_selected_body_tail_preserves_closer() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app_at(directory.path(), "*italic*\n\nNEXT", 3..7);
    let ctx = Context::default();
    frame(&mut app, &ctx, true, vec![]);
    frame(&mut app, &ctx, true, vec![egui::Event::Text("X".into())]);
    assert_eq!(app.session[0].content, "*itX*\n\nNEXT");
}

#[test]
fn product_input_newline_repeated_enter_then_text_preserves_insertion_order() {
    let mut failures = Vec::new();
    for hybrid in [false, true] {
        for (source, cursor) in [
            ("", 0),
            ("   ", 1),
            ("   ", 3),
            ("FIRST", 2),
            ("FIRST", 5),
            ("FIRST\n\nNEXT", 2),
            ("FIRST\n\nNEXT", 5),
            ("FIRST\n\n\nNEXT", 5),
            ("FIRST\n\nLAST", 11),
            ("FIRST\n\nLAST\n\n", 11),
            ("甲🙂乙\n\n尾段", 2),
        ] {
            for count in 1..=3 {
                let directory = tempfile::tempdir().unwrap();
                let mut app = app_at(directory.path(), source, cursor..cursor);
                let ctx = Context::default();
                frame(&mut app, &ctx, hybrid, vec![]);
                for _ in 0..count {
                    frame(
                        &mut app,
                        &ctx,
                        hybrid,
                        vec![key(Key::Enter, egui::Modifiers::NONE)],
                    );
                }
                let before_typing = app.session[0].content.clone();
                frame(
                    &mut app,
                    &ctx,
                    hybrid,
                    vec![egui::Event::Text("NEW".into())],
                );
                let at = char_to_byte(source, cursor);
                let expected = format!("{}NEW{}", &source[..at], &source[at..]).replace('\n', "");
                if app.session[0].content.replace('\n', "") != expected {
                    failures.push(format!("hybrid={hybrid} source={source:?} cursor={cursor} count={count} before_typing={before_typing:?} expected={expected:?} actual={:?}", app.session[0].content));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_newline_enter_then_backspace_restores_original() {
    let mut failures = Vec::new();
    for hybrid in [false, true] {
        for (source, cursor) in [
            ("", 0),
            ("   ", 1),
            ("   ", 3),
            ("FIRST", 2),
            ("FIRST", 5),
            ("FIRST\n\nNEXT", 2),
            ("FIRST\n\nNEXT", 5),
            ("FIRST\n\n\nNEXT", 5),
            ("FIRST\n\nLAST", 11),
            ("FIRST\n\nLAST\n\n", 11),
            ("甲🙂乙\n\n尾段", 2),
        ] {
            for count in 1..=3 {
                let directory = tempfile::tempdir().unwrap();
                let mut app = app_at(directory.path(), source, cursor..cursor);
                let ctx = Context::default();
                frame(&mut app, &ctx, hybrid, vec![]);
                for _ in 0..count {
                    frame(
                        &mut app,
                        &ctx,
                        hybrid,
                        vec![key(Key::Enter, egui::Modifiers::NONE)],
                    );
                }
                let entered = app.session[0].content.clone();
                for _ in 0..count {
                    frame(
                        &mut app,
                        &ctx,
                        hybrid,
                        vec![key(Key::Backspace, egui::Modifiers::NONE)],
                    );
                }
                if app.session[0].content != source {
                    failures.push(format!("hybrid={hybrid} source={source:?} cursor={cursor} count={count} entered={entered:?} actual={:?}", app.session[0].content));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_newline_shift_enter_retains_hard_break_and_following_text() {
    let mut failures = Vec::new();
    for (source, cursor) in [
        ("FIRST", 2),
        ("FIRST", 5),
        ("FIRST\n\nNEXT", 2),
        ("FIRST\n\nNEXT", 5),
        ("FIRST \n\nNEXT", 6),
        ("FIRST  \n\nNEXT", 7),
        ("FIRST\\\n\nNEXT", 6),
        ("```\nFIRST\n```", 9),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), source, cursor..cursor);
        let ctx = Context::default();
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![key(Key::Enter, egui::Modifiers::SHIFT)],
        );
        let entered = app.session[0].content.clone();
        frame(&mut app, &ctx, true, vec![egui::Event::Text("NEW".into())]);
        let at = char_to_byte(source, cursor);
        let expected = format!("{}NEW{}", &source[..at], &source[at..]);
        let letters = |text: &str| {
            text.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        };
        let code = source.starts_with("```");
        let rendered = markdown::render_html_fragment(&app.session[0].content);
        let has_break = if code {
            fenced_code_content(&app.session[0].content)
                .is_some_and(|body| body.contains("FIRST\nNEW"))
        } else {
            rendered.contains("<br")
        };
        if letters(&app.session[0].content) != letters(&expected) || !has_break {
            failures.push(format!("source={source:?} cursor={cursor} entered={entered:?} expected={expected:?} actual={:?}", app.session[0].content));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_newline_block_types_delete_and_undo_redo_roundtrip() {
    let mut failures = Vec::new();
    for body in [
        "PARAGRAPH",
        "# HEADING",
        "- ITEM",
        "1. ITEM",
        "> QUOTE",
        "```text\nCODE\n```",
    ] {
        for tail in ["", "\n\nNEXT"] {
            for count in 1..=3 {
                for deletion in [Key::Backspace, Key::Delete] {
                    let source = format!("{body}{tail}");
                    let cursor = if body.starts_with("```") {
                        "```text\nCODE".chars().count()
                    } else {
                        body.chars().count()
                    };
                    let directory = tempfile::tempdir().unwrap();
                    let mut app = app_at(directory.path(), &source, cursor..cursor);
                    let ctx = Context::default();
                    let mut fonts = FontDefinitions::default();
                    fonts.families.insert(
                        FontFamily::Name(WYSIWYG_STRONG_FAMILY.into()),
                        fonts.families[&FontFamily::Proportional].clone(),
                    );
                    ctx.set_fonts(fonts);
                    frame(&mut app, &ctx, true, vec![]);
                    for _ in 0..count {
                        frame(
                            &mut app,
                            &ctx,
                            true,
                            vec![key(Key::Enter, egui::Modifiers::NONE)],
                        );
                    }
                    frame(&mut app, &ctx, true, vec![egui::Event::Text("NEW".into())]);
                    frame(
                        &mut app,
                        &ctx,
                        true,
                        vec![key(deletion, egui::Modifiers::NONE)],
                    );
                    let edited = app.session[0].content.clone();
                    let mut undo_count = 0;
                    while app.session[0].can_undo() && undo_count < 20 {
                        app.undo_active();
                        undo_count += 1;
                    }
                    if app.session[0].content != source {
                        failures.push(format!("undo body={body:?} tail={tail:?} enters={count} key={deletion:?} actual={:?}", app.session[0].content));
                    }
                    for _ in 0..undo_count {
                        app.redo_active();
                    }
                    if app.session[0].content != edited {
                        failures.push(format!("redo body={body:?} tail={tail:?} enters={count} key={deletion:?} expected={edited:?} actual={:?}", app.session[0].content));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_newline_batches_match_individual_key_frames() {
    let mut failures = Vec::new();
    for (source, cursor) in [
        ("FIRST\n\nNEXT", 2),
        ("FIRST\n\nNEXT", 5),
        ("- ITEM", 6),
        ("```\nCODE\n```", 8),
    ] {
        for events in [
            vec![
                key(Key::Enter, egui::Modifiers::NONE),
                egui::Event::Text("NEW".into()),
            ],
            vec![
                egui::Event::Text("X".into()),
                key(Key::Enter, egui::Modifiers::NONE),
                egui::Event::Text("NEW".into()),
            ],
            vec![
                key(Key::Enter, egui::Modifiers::NONE),
                key(Key::Enter, egui::Modifiers::NONE),
                key(Key::Backspace, egui::Modifiers::NONE),
                egui::Event::Text("NEW".into()),
            ],
            vec![
                key(Key::Enter, egui::Modifiers::SHIFT),
                egui::Event::Text("NEW".into()),
            ],
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut separate = app_at(directory.path(), source, cursor..cursor);
            let ctx = Context::default();
            frame(&mut separate, &ctx, true, vec![]);
            for event in &events {
                frame(&mut separate, &ctx, true, vec![event.clone()]);
            }
            let mut batched = app_at(directory.path(), source, cursor..cursor);
            let ctx = Context::default();
            frame(&mut batched, &ctx, true, vec![]);
            frame(&mut batched, &ctx, true, events.clone());
            if separate.session[0].content != batched.session[0].content {
                failures.push(format!(
                    "source={source:?} cursor={cursor} events={events:?} separate={:?} batch={:?}",
                    separate.session[0].content, batched.session[0].content
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_input_newline_splits_formatted_paragraph_without_exposing_markers() {
    for (source, cursor, expected) in [
        (
            "**FIRST**",
            4,
            "<p><strong>FI</strong></p>\n<p><strong>RST</strong></p>\n",
        ),
        ("*FIRST*", 3, "<p><em>FI</em></p>\n<p><em>RST</em></p>\n"),
        (
            "`FIRST`",
            3,
            "<p><code>FI</code></p>\n<p><code>RST</code></p>\n",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_at(directory.path(), source, cursor..cursor);
        let ctx = Context::default();
        let mut fonts = FontDefinitions::default();
        fonts.families.insert(
            FontFamily::Name(WYSIWYG_STRONG_FAMILY.into()),
            fonts.families[&FontFamily::Proportional].clone(),
        );
        ctx.set_fonts(fonts);
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![key(Key::Enter, egui::Modifiers::NONE)],
        );
        assert_eq!(
            markdown::render_html_fragment(&app.session[0].content),
            expected,
            "source={source:?}, actual={:?}",
            app.session[0].content
        );
        frame(
            &mut app,
            &ctx,
            true,
            vec![key(Key::Backspace, egui::Modifiers::NONE)],
        );
        assert_eq!(
            app.session[0].content, source,
            "joining the split formatted paragraph must restore its markup"
        );
    }
}

#[test]
fn product_input_newline_empty_code_removal_preserves_later_paragraphs() {
    for gap in ["\n\n", "\n\n\n", "\n\n\n\n"] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("```\n\n```{gap}NEXT\n\nLAST");
        let mut app = app_at(directory.path(), &source, 4..4);
        let ctx = Context::default();
        frame(&mut app, &ctx, true, vec![]);
        frame(
            &mut app,
            &ctx,
            true,
            vec![key(Key::Backspace, egui::Modifiers::NONE)],
        );
        assert!(
            app.session[0].content.ends_with("NEXT\n\nLAST"),
            "gap={gap:?}, actual={:?}",
            app.session[0].content
        );
        assert!(!app.session[0].content.contains("```"));
    }
}
