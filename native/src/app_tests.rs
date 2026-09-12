//! Shell integration regressions: menus, file lifecycle and editor wiring.
fn code_and_following_paragraph_rects(
    app: &mut RuporaApp,
    context: &Context,
) -> (egui::Rect, egui::Rect) {
    fn collect(
        shape: &egui::Shape,
        code: &mut Option<egui::Rect>,
        paragraph: &mut Option<egui::Rect>,
    ) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, code, paragraph);
                }
            }
            egui::Shape::Rect(shape) if shape.fill == app_palette(false).code_bg => {
                *code = Some(shape.rect);
            }
            egui::Shape::Text(shape) if shape.galley.job.text == "wdadawd" => {
                *paragraph = Some(shape.galley.rect.translate(shape.pos.to_vec2()));
            }
            _ => {}
        }
    }
    let output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            ..Default::default()
        },
        |ui| app.hybrid_pane(ui, 0),
    );
    let (mut code, mut paragraph) = (None, None);
    for shape in output.shapes {
        collect(&shape.shape, &mut code, &mut paragraph);
    }
    (
        code.expect("code surface must be painted"),
        paragraph.expect("following paragraph must be painted"),
    )
}
use super::*;

#[test]
fn document_canvases_keep_text_inside_narrow_panes_without_changing_source() {
    fn check(shape: &egui::Shape, clip: egui::Rect, mode: ViewMode, width: f32) {
        match shape {
            egui::Shape::Text(text) => {
                let bounds = text.galley.rect.translate(text.pos.to_vec2());
                assert!(
                    bounds.left() >= clip.left() - 1.0 && bounds.right() <= clip.right() + 1.0,
                    "{mode:?} width={width}: {:?} outside {clip:?}: {:?}",
                    bounds,
                    text.galley.job.text
                );
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    check(shape, clip, mode, width);
                }
            }
            _ => {}
        }
    }
    let source = "# 安静地写作\n\n中文 English 🙂 and a long paragraph which wraps across the available page width.\n\n```rust\nprintln!(\"中文代码 🙂\");\n```\n\nAFTER CODE";
    let context = Context::default();
    install_fonts(&context);
    apply_theme(&context, false);
    for mode in [ViewMode::Edit, ViewMode::Preview, ViewMode::Hybrid] {
        for width in [180.0, 340.0, 600.0] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = isolated_app(directory.path());
            app.new_document();
            app.session[0].content = source.to_owned();
            app.session[0].update_after_edit();
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 560.0),
                    )),
                    ..Default::default()
                },
                |ui| app.show_editor_pane(ui, 0, mode),
            );
            for shape in output.shapes {
                check(&shape.shape, shape.clip_rect, mode, width);
            }
            assert_eq!(app.session[0].content, source);
        }
    }
}

#[test]
fn split_document_surfaces_start_at_the_same_height() {
    fn collect(shape: &egui::Shape, tops: &mut [f32; 2]) {
        match shape {
            egui::Shape::Rect(rect) if rect.fill == app_palette(false).surface => {
                let column = usize::from(rect.rect.center().x > 450.0);
                tops[column] = tops[column].min(rect.rect.top());
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, tops);
                }
            }
            _ => {}
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].content = "# Heading\n\nText".to_owned();
    app.session[0].update_after_edit();
    let context = Context::default();
    install_fonts(&context);
    let output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 640.0),
            )),
            ..Default::default()
        },
        |ui| app.show_editor_pane(ui, 0, ViewMode::Split),
    );
    let mut tops = [f32::INFINITY; 2];
    for shape in output.shapes {
        collect(&shape.shape, &mut tops);
    }
    assert!(
        tops[0].is_finite() && tops[1].is_finite(),
        "both page surfaces must be painted"
    );
    assert!(
        (tops[0] - tops[1]).abs() <= 1.0,
        "split page tops differ: {tops:?}"
    );
}

#[test]
fn workspace_identity_and_status_stay_left_aligned_at_different_widths() {
    fn collect(shape: &egui::Shape, positions: &mut Vec<(String, f32)>) {
        match shape {
            egui::Shape::Text(text) => positions.push((text.galley.job.text.clone(), text.pos.x)),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, positions);
                }
            }
            _ => {}
        }
    }
    for width in [820.0, 1440.0] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.status = "已保存".to_owned();
        let title = app.session[0].title();
        let context = Context::default();
        install_fonts(&context);
        apply_theme(&context, false);
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 560.0),
                )),
                ..Default::default()
            },
            |ui| {
                app.top_bar(ui);
                app.status_bar(ui);
            },
        );
        let mut positions = Vec::new();
        for shape in output.shapes {
            collect(&shape.shape, &mut positions);
        }
        for label in [&title, "开始新的文稿", "已保存"] {
            let x = positions
                .iter()
                .find(|(text, _)| text == label)
                .expect("shell label must be visible")
                .1;
            assert!(
                (x - 24.0).abs() <= 1.0,
                "width={width}: {label:?} starts at {x}, expected the left inset"
            );
        }
    }
}

#[test]
fn full_audit_closing_a_saved_document_removes_its_old_recovery_entry() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].edit(EditKind::Other, None, |content| {
        content.push_str("draft");
        None
    });
    app.recovery_store.save(app.session.documents()).unwrap();
    assert_eq!(app.recovery_store.load().unwrap().len(), 1);
    app.session[0]
        .save_as(directory.path().join("saved.md"), false)
        .unwrap();
    app.close_document(0);
    assert!(app.recovery_store.load().unwrap().is_empty());
}

#[test]
fn full_audit_generated_toc_links_navigate_to_literal_heading_ids() {
    for id in ["part)", "%61", "#part", "中文"] {
        let source = format!("[TOC]\n\n# Target {{#{id}}}\n\nEND");
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.session[0].content = source.clone();
        app.session[0].update_after_edit();
        let toc = markdown::toc_preview_markdown(&source);
        let destination = pulldown_cmark::Parser::new_ext(&toc, markdown::parser_options())
            .find_map(|event| match event {
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { dest_url, .. }) => {
                    Some(dest_url.into_string())
                }
                _ => None,
            })
            .unwrap();
        app.open_preview_destination(0, &destination);
        assert_eq!(
            app.editor_surface.cursor().unwrap().primary.index.0,
            source[..source.find("# Target").unwrap()].chars().count(),
            "id={id:?} destination={destination:?}"
        );
        assert_eq!(app.session[0].content, source);
    }
}

#[test]
fn full_audit_activating_an_overflowed_tab_reveals_its_title() {
    fn visible(shape: &egui::Shape, clip: egui::Rect, title: &str) -> bool {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == title => {
                clip.contains_rect(text.galley.rect.translate(text.pos.to_vec2()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| visible(shape, clip, title)),
            _ => false,
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    for _ in 0..12 {
        app.new_document();
    }
    let ctx = Context::default();
    let mut time = 0.0;
    for index in [11, 0, 9] {
        app.activate_document(index);
        let title = app.session[index].title();
        let mut found = false;
        for _ in 0..8 {
            time += 1.0;
            let output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(500.0, 700.0),
                    )),
                    ..Default::default()
                },
                |ui| app.editor_tabs(ui),
            );
            found |= output
                .shapes
                .iter()
                .any(|shape| visible(&shape.shape, shape.clip_rect, &title));
        }
        assert!(found, "active tab {title:?} is outside the visible strip");
    }
}

#[test]
fn extension_result_is_rejected_after_save_as_changes_document_context() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    let source = directory.path().join("first.md");
    fs::write(&source, "original").unwrap();
    let id = app.session.insert(Document::open(&source).unwrap());
    let job = ExtensionJobResult {
        token: app.session.get(id).unwrap().snapshot_token(),
        invocation: crate::extensions::ExtensionInvocation {
            replacement: Some("result based on the old path".to_owned()),
            message: None,
        },
    };
    app.session
        .get_mut(id)
        .unwrap()
        .save_as(directory.path().join("second.md"), false)
        .unwrap();
    app.apply_extension_result(Ok(job));
    assert_eq!(app.session.get(id).unwrap().content, "original");
    assert!(!app.session.get(id).unwrap().dirty);
    assert!(app.status.contains("过期"));
}

fn isolated_app(directory: &Path) -> RuporaApp {
    RuporaApp::from_state(
        PersistedState::default(),
        RecoveryStore::at(directory.join("recovery.json")),
        None,
        ExtensionRegistry::disabled(directory.join("extensions.json")),
        None,
    )
}

#[test]
fn table_snapshot_rejects_edits_even_after_the_original_text_is_restored() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.insert_text("| A |\n| --- |\n| B |\n", EditKind::Other);
    app.queue_editor_selection(2..2);
    app.open_table_editor();
    app.table_editor.as_mut().unwrap().table.headers[0] = "stale".to_owned();
    let before = app.session[0].content.clone();
    app.insert_text("edit", EditKind::Other);
    app.undo_active();
    assert_eq!(app.session[0].content, before);
    app.apply_table_editor();
    assert_eq!(app.session[0].content, before);
    assert!(app.status.contains("已发生变化"));
    assert!(app.session[0].can_redo());
    assert_eq!(app.table_editor.as_ref().unwrap().table.headers[0], "stale");
}

#[test]
fn table_snapshot_rejects_an_invalid_byte_range_without_mutating_document() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.insert_text("中文🙂", EditKind::Other);
    app.open_table_editor();
    app.table_editor.as_mut().unwrap().table.range = 1..2;
    app.apply_table_editor();
    assert_eq!(app.session[0].content, "中文🙂");
    assert!(app.status.contains("范围已失效"));
    assert!(app.table_editor.is_some());
}

#[test]
fn a_closed_table_target_keeps_the_draft_available_for_copying() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.open_table_editor();
    app.table_editor.as_mut().unwrap().table.headers[0] = "保留草稿🙂".to_owned();
    let id = app.session[0].id();
    app.session.remove(id);
    app.apply_table_editor();
    assert!(app.status.contains("已关闭"));
    assert_eq!(
        app.table_editor.as_ref().unwrap().table.headers[0],
        "保留草稿🙂"
    );
}

#[test]
fn successful_background_reload_does_not_hide_another_documents_scan_error() {
    for reverse in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        let id = app.session[0].id();
        let mut events = vec![
            ExternalEvent::Error {
                document_id: id,
                error: "无法读取文档".to_owned(),
            },
            ExternalEvent::Reloaded {
                document_id: id,
                path: directory.path().join("healthy.md"),
            },
        ];
        if reverse {
            events.reverse();
        }
        app.apply_external_events(events);
        assert_eq!(app.status, "无法读取文档");
    }
}

#[test]
fn extension_commit_targets_its_original_tab_and_can_be_undone_once() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.insert_text("target", EditKind::Other);
    let token = app.session[0].snapshot_token();
    app.new_document();
    app.insert_text("keep", EditKind::Other);
    app.apply_extension_result(Ok(ExtensionJobResult {
        token,
        invocation: crate::extensions::ExtensionInvocation {
            replacement: Some("更新🙂".to_owned()),
            message: None,
        },
    }));
    assert_eq!(app.session.active_id(), Some(token.document_id()));
    assert_eq!(app.session[0].content, "更新🙂");
    assert_eq!(app.session[1].content, "keep");
    app.undo_active();
    assert_eq!(app.session[0].content, "target");
    app.redo_active();
    assert_eq!(app.session[0].content, "更新🙂");
}

fn hybrid_test_frame(
    app: &mut RuporaApp,
    context: &Context,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> egui::FullOutput {
    context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            events,
            modifiers,
            ..Default::default()
        },
        |ui| app.hybrid_pane(ui, 0),
    )
}

#[test]
fn source_audit_cross_block_copy_then_type_and_cut_paste_keep_the_full_range() {
    for cut in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        let source = "one\n\ntwo  \n\nthree";
        let selected = "one\n\ntwo  \n\n";
        app.session[0].content = source.to_owned();
        app.session[0].update_after_edit();
        app.queue_editor_selection(0..selected.chars().count());
        let context = Context::default();
        hybrid_test_frame(&mut app, &context, vec![], egui::Modifiers::NONE);
        if cut {
            let frame = hybrid_test_frame(
                &mut app,
                &context,
                vec![egui::Event::Cut],
                egui::Modifiers::NONE,
            );
            let copied = frame
                .platform_output
                .commands
                .into_iter()
                .find_map(|command| match command {
                    egui::OutputCommand::CopyText(text) => Some(text),
                    _ => None,
                })
                .expect("cut must copy the selected Markdown");
            hybrid_test_frame(
                &mut app,
                &context,
                vec![egui::Event::Paste(copied)],
                egui::Modifiers::NONE,
            );
            assert_eq!(app.session[0].content, source);
        } else {
            hybrid_test_frame(
                &mut app,
                &context,
                vec![egui::Event::Copy, egui::Event::Text("X".to_owned())],
                egui::Modifiers::NONE,
            );
            assert_eq!(app.session[0].content, "Xthree");
        }
    }
}

#[test]
fn source_audit_pasted_line_endings_are_normalized_in_both_editors() {
    for hybrid in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.queue_editor_selection(0..0);
        let context = Context::default();
        for events in [vec![], vec![egui::Event::Paste("甲\r\n乙\r丙".to_owned())]] {
            let _ = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    if hybrid {
                        app.hybrid_pane(ui, 0);
                    } else {
                        app.edit_pane(ui, 0, None);
                    }
                },
            );
        }
        assert_eq!(app.session[0].content, "甲\n乙\n丙", "hybrid={hybrid}");
    }
}

#[test]
fn source_audit_batched_input_preserves_every_event_in_both_editors() {
    for hybrid in [false, true] {
        for (source, selection, events, expected) in [
            (
                "",
                0..0,
                vec![
                    egui::Event::Text("中文".to_owned()),
                    egui::Event::Text("(".to_owned()),
                ],
                "中文(",
            ),
            (
                "AB",
                2..2,
                vec![
                    egui::Event::Key {
                        key: Key::Backspace,
                        physical_key: Some(Key::Backspace),
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Text("(".to_owned()),
                ],
                "A(",
            ),
            (
                "旧文🙂",
                0..3,
                vec![
                    egui::Event::Paste("https://example.com".to_owned()),
                    egui::Event::Text("!".to_owned()),
                ],
                "https://example.com!",
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = isolated_app(directory.path());
            app.new_document();
            app.session[0].content = source.to_owned();
            app.session[0].update_after_edit();
            app.queue_editor_selection(selection);
            let context = Context::default();
            for events in [vec![], events] {
                let _ = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 800.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        if hybrid {
                            app.hybrid_pane(ui, 0);
                        } else {
                            app.edit_pane(ui, 0, None);
                        }
                    },
                );
            }
            assert_eq!(
                app.session[0].content, expected,
                "hybrid={hybrid}, source={source:?}"
            );
            app.undo_active();
            assert_eq!(app.session[0].content, source);
        }
    }
}

#[test]
fn source_audit_source_enter_does_not_continue_markdown_inside_code() {
    for body in ["- literal", "1. literal", "> literal"] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        let before = format!("```text\n{body}");
        app.session[0].content = format!("{before}\n```");
        app.session[0].update_after_edit();
        let cursor = before.chars().count();
        app.queue_editor_selection(cursor..cursor);
        let context = Context::default();
        for events in [
            vec![],
            vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let _ = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.edit_pane(ui, 0, None);
                },
            );
        }
        assert_eq!(app.session[0].content, format!("{before}\n\n```"));
    }
}

#[test]
fn source_audit_programmatic_cross_block_selection_survives_rendering() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].content = "one\n\ntwo".to_owned();
    app.session[0].update_after_edit();
    app.queue_editor_selection(1..7);
    let context = Context::default();
    hybrid_test_frame(&mut app, &context, vec![], egui::Modifiers::NONE);
    hybrid_test_frame(
        &mut app,
        &context,
        vec![egui::Event::Text("X".to_owned())],
        egui::Modifiers::NONE,
    );
    assert_eq!(app.session[0].content, "oXo");
    app.undo_active();
    assert_eq!(app.session[0].content, "one\n\ntwo");
}

fn audit_painted_text(output: &egui::FullOutput) -> String {
    fn append(shape: &egui::Shape, text: &mut String) {
        match shape {
            egui::Shape::Text(shape) => {
                text.push_str(&shape.galley.job.text);
                text.push('\n');
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| append(shape, text)),
            _ => {}
        }
    }
    let mut text = String::new();
    output
        .shapes
        .iter()
        .for_each(|shape| append(&shape.shape, &mut text));
    text
}

#[test]
fn source_audit_auxiliary_text_input_undo_does_not_change_the_document() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.insert_text("Document content", EditKind::Typing);
    let context = Context::default();
    let id = egui::Id::new("audit-find-field");
    let mut query = String::from("query");
    for events in [
        vec![],
        vec![egui::Event::Text("X".to_owned())],
        vec![egui::Event::Key {
            key: Key::Z,
            physical_key: Some(Key::Z),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    ] {
        let _ = context.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                app.handle_shortcuts(&context);
                ui.memory_mut(|memory| memory.request_focus(id));
                TextEdit::singleline(&mut query).id(id).show(ui);
            },
        );
    }
    assert_eq!(app.session[0].content, "Document content");
    assert_eq!(query, "query");
}

#[test]
fn source_audit_table_apply_after_switching_tabs_targets_the_original_document() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.insert_text("| A |\n| --- |\n| B |\n", EditKind::Typing);
    app.queue_editor_selection(2..2);
    app.open_table_editor();
    app.table_editor.as_mut().unwrap().table.headers[0] = "Edited".to_owned();
    app.new_document();
    app.insert_text("Keep this tab", EditKind::Typing);
    app.apply_table_editor();
    assert_eq!(app.session.active_index(), Some(0));
    assert!(app.session[0].content.contains("Edited"));
    let context = Context::default();
    install_fonts(&context);
    hybrid_test_frame(&mut app, &context, vec![], egui::Modifiers::NONE);
    hybrid_test_frame(
        &mut app,
        &context,
        vec![egui::Event::Text("More".to_owned())],
        egui::Modifiers::NONE,
    );
    assert_eq!(app.session[1].content, "Keep this tab");
    assert!(app.session[0].content.contains("More"));
}

#[test]
fn source_audit_command_palette_enter_executes_the_selected_command() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.command_palette_open = true;
    app.command_focus_requested = true;
    app.command_query = "新建文档".to_owned();
    let context = Context::default();
    install_fonts(&context);
    for events in [
        vec![],
        vec![],
        vec![egui::Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    ] {
        let _ = context.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| app.command_palette(ui),
        );
    }
    assert_eq!(app.session.documents().len(), 2);
    assert!(!app.command_palette_open);
}

#[test]
fn source_audit_reading_anchor_navigation_scrolls_the_target_into_view() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].content = format!("{}\n# Target\n\nTail", "Paragraph\n\n".repeat(60));
    app.session[0].update_after_edit();
    app.state.view_mode = ViewMode::Preview;
    let context = Context::default();
    install_fonts(&context);
    let frame = |app: &mut RuporaApp, time: f64| {
        context.run_ui(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                app.preview_pane(ui, 0, None);
            },
        )
    };
    let _ = frame(&mut app, 0.0);
    app.jump_to_anchor(0, "target");
    let _ = frame(&mut app, 1.0);
    let _ = frame(&mut app, 10.0);
    let output = frame(&mut app, 11.0);
    assert!(
        output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Target" => shape
                .clip_rect
                .intersects(text.galley.rect.translate(text.pos.to_vec2())),
            _ => false,
        }),
        "anchor target must be inside the reading viewport"
    );
    assert_eq!(app.state.view_mode, ViewMode::Preview);
}

#[test]
fn source_audit_command_enter_is_not_replayed_in_the_newly_focused_editor() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].content = "KEEP".to_owned();
    app.session[0].update_after_edit();
    app.editor_surface
        .select_cursor(CCursorRange::one(CCursor::new(2)));
    app.state.view_mode = ViewMode::Preview;
    app.command_palette_open = true;
    app.command_focus_requested = true;
    app.command_query = "切换到源码模式".to_owned();
    let context = Context::default();
    install_fonts(&context);
    for events in [
        vec![],
        vec![],
        vec![egui::Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    ] {
        let _ = context.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                app.command_palette(ui);
                app.editor(ui);
            },
        );
    }
    assert_eq!(app.state.view_mode, ViewMode::Edit);
    assert_eq!(app.session[0].content, "KEEP");
}

#[test]
fn source_audit_find_enter_does_not_replace_the_match_with_a_newline() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].content = "KEEP".to_owned();
    app.session[0].update_after_edit();
    app.editor_surface
        .select_cursor(CCursorRange::one(CCursor::new(0)));
    app.state.view_mode = ViewMode::Edit;
    app.find_open = true;
    app.find_focus_requested = true;
    app.find_query = "EP".to_owned();
    let context = Context::default();
    install_fonts(&context);
    for events in [
        vec![],
        vec![],
        vec![egui::Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    ] {
        let _ = context.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                app.find_bar(ui);
                app.editor(ui);
            },
        );
    }
    assert_eq!(app.session[0].content, "KEEP");
    assert_eq!(app.active_selection(0), 2..4);
}

#[test]
fn source_audit_toc_and_metadata_render_in_both_preview_modes() {
    for hybrid in [true, false] {
        for (source, expected) in [
            ("[TOC]\n\n# Heading", "• Heading"),
            ("---\ntitle: Sample\n---\n\n# Heading", "文档元数据"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = isolated_app(directory.path());
            app.new_document();
            app.session[0].content = source.to_owned();
            app.session[0].update_after_edit();
            app.editor_surface.clear_selection();
            app.editor_surface.invalidate_content();
            let context = Context::default();
            install_fonts(&context);
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                if hybrid {
                    app.hybrid_pane(ui, 0);
                } else {
                    app.preview_pane(ui, 0, None);
                }
            });
            let painted = audit_painted_text(&output);
            assert!(painted.contains(expected), "hybrid={hybrid}: {painted:?}");
        }
    }
}

#[test]
fn source_audit_drag_over_generated_toc_uses_original_source_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].content = "Intro\n\n[TOC]\n\n# A considerably longer heading".to_owned();
    app.session[0].update_after_edit();
    app.editor_surface.clear_selection();
    app.editor_surface.invalidate_content();
    let context = Context::default();
    install_fonts(&context);
    let output = hybrid_test_frame(&mut app, &context, vec![], egui::Modifiers::NONE);
    let position = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.starts_with("• A considerably") => {
                Some(text.pos + text.galley.rect.right_center().to_vec2())
            }
            _ => None,
        })
        .expect("painted table of contents");
    hybrid_test_frame(
        &mut app,
        &context,
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        egui::Modifiers::NONE,
    );
    assert_eq!(
        app.session[0].content,
        "Intro\n\n[TOC]\n\n# A considerably longer heading"
    );
}

#[test]
fn extreme_backspace_at_code_start_moves_to_the_previous_paragraph() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    let source = "前文🙂\n\n```rust\ncode\n```\n\nKEEP";
    app.session[0].content = source.to_owned();
    app.session[0].update_after_edit();
    let cursor = "前文🙂\n\n```rust\n".chars().count();
    app.queue_editor_selection(cursor..cursor);
    let context = Context::default();
    hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
    hybrid_test_frame(
        &mut app,
        &context,
        vec![egui::Event::Key {
            key: Key::Backspace,
            physical_key: Some(Key::Backspace),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        egui::Modifiers::NONE,
    );
    assert_eq!(app.session[0].content, source);
    hybrid_test_frame(
        &mut app,
        &context,
        vec![egui::Event::Text("接续".to_owned())],
        egui::Modifiers::NONE,
    );
    assert_eq!(
        app.session[0].content,
        "前文🙂接续\n\n```rust\ncode\n```\n\nKEEP"
    );
}

#[test]
fn extreme_replacing_selected_separators_keeps_the_following_code_block() {
    for (event, replacement) in [
        (
            egui::Event::Key {
                key: Key::Delete,
                physical_key: Some(Key::Delete),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            "",
        ),
        (egui::Event::Cut, ""),
        (egui::Event::Paste("替换🙂".to_owned()), "替换🙂"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        let source = "正文🙂\n\n```rust\ncode\n```\n\nKEEP";
        app.session[0].content = source.to_owned();
        app.session[0].update_after_edit();
        app.queue_editor_selection(3..3);
        let context = Context::default();
        hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
        // Ctrl+Shift+End now intentionally selects to the document end.
        // Keep this regression focused on replacing only the separators.
        app.queue_editor_selection(3..5);
        hybrid_test_frame(&mut app, &context, vec![], egui::Modifiers::NONE);
        hybrid_test_frame(&mut app, &context, vec![event], egui::Modifiers::NONE);
        assert_eq!(
            app.session[0].content,
            format!("正文🙂{replacement}\n```rust\ncode\n```\n\nKEEP")
        );
        app.undo_active();
        assert_eq!(app.session[0].content, source);
    }
}

#[test]
fn extreme_backspace_after_a_code_block_preserves_its_fence_boundary() {
    for gap in ["\n", "\n\n"] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        let code = "```rust\ncode🙂\n```";
        let source = format!("{code}{gap}正文\n\nKEEP");
        app.session[0].content = source;
        app.session[0].update_after_edit();
        let cursor = code.chars().count() + gap.len();
        app.queue_editor_selection(cursor..cursor);
        let context = Context::default();
        hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
        for _ in 0..gap.len() {
            hybrid_test_frame(
                &mut app,
                &context,
                vec![egui::Event::Key {
                    key: Key::Backspace,
                    physical_key: Some(Key::Backspace),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
        }
        assert_eq!(app.session[0].content, format!("{code}\n正文\n\nKEEP"));
        assert_eq!(
            app.editor_surface.cursor().unwrap().primary.index.0,
            "```rust\ncode🙂".chars().count()
        );
    }
}

#[test]
fn extreme_delete_before_a_code_block_preserves_its_fence_boundary() {
    for gap in ["\n", "\n\n"] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        let source = format!("正文🙂{gap}```rust\ncode\n```\n\nKEEP");
        app.session[0].content = source.clone();
        app.session[0].update_after_edit();
        app.queue_editor_selection(3..3);
        let context = Context::default();
        hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
        for _ in 0..gap.len() {
            hybrid_test_frame(
                &mut app,
                &context,
                vec![egui::Event::Key {
                    key: Key::Delete,
                    physical_key: Some(Key::Delete),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
        }
        assert_eq!(app.session[0].content, "正文🙂\n```rust\ncode\n```\n\nKEEP");
        assert_eq!(
            app.editor_surface.cursor().unwrap().primary.index.0,
            "正文🙂\n```rust\n".chars().count()
        );
    }
}

#[test]
fn human_double_click_after_an_active_code_block_creates_a_paragraph() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    let source = "```text\ncode\n```";
    app.session[0].content = source.to_owned();
    app.session[0].update_after_edit();
    app.queue_editor_selection(10..10);
    let context = Context::default();
    let frame = hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
    let code_rect = frame
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == app_palette(false).code_bg => Some(rect.rect),
            _ => None,
        })
        .expect("code block must be rendered");
    let position = code_rect.center_bottom() + egui::vec2(0.0, 15.0);
    for pressed in [true, false, true, false] {
        hybrid_test_frame(
            &mut app,
            &context,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            egui::Modifiers::NONE,
        );
    }
    assert_eq!(app.session[0].content, format!("{source}\n\n"));
    hybrid_test_frame(
        &mut app,
        &context,
        vec![egui::Event::Text("正文".to_owned())],
        egui::Modifiers::NONE,
    );
    assert_eq!(app.session[0].content, format!("{source}\n\n正文"));
}

#[test]
fn human_select_all_and_replace_a_paragraph_preserves_the_next_block() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    let source = "正文🙂\n\n- [ ] 后面的任务";
    app.session[0].content = source.to_owned();
    app.session[0].update_after_edit();
    app.queue_editor_selection(1..1);
    let context = Context::default();
    hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
    let modifiers = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    hybrid_test_frame(
        &mut app,
        &context,
        vec![egui::Event::Key {
            key: Key::A,
            physical_key: Some(Key::A),
            pressed: true,
            repeat: false,
            modifiers,
        }],
        modifiers,
    );
    hybrid_test_frame(
        &mut app,
        &context,
        vec![egui::Event::Text("替换".to_owned())],
        egui::Modifiers::NONE,
    );
    assert_eq!(app.session[0].content, "替换\n\n- [ ] 后面的任务");
    app.undo_active();
    assert_eq!(app.session[0].content, source);
}

#[test]
fn human_typing_a_language_fence_then_enter_keeps_code_out_of_the_info_string() {
    for opener in ["```python", "~~~rust", "````text"] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.queue_editor_selection(0..0);
        let context = Context::default();
        hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
        for event in [
            egui::Event::Text(opener.to_owned()),
            egui::Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Text("print(\"你好🙂\")".to_owned()),
        ] {
            hybrid_test_frame(&mut app, &context, vec![event], egui::Modifiers::NONE);
        }
        let fence = opener
            .chars()
            .take_while(|c| *c == '`' || *c == '~')
            .collect::<String>();
        assert_eq!(
            app.session[0].content,
            format!("{opener}\nprint(\"你好🙂\")\n{fence}")
        );
    }
}

#[test]
fn human_clicking_an_active_task_checkbox_toggles_it_and_can_be_undone() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    let source = "- [ ] 第一项\n- [ ] 第二项";
    app.session[0].content = source.to_owned();
    app.session[0].update_after_edit();
    let cursor = source.chars().count();
    app.queue_editor_selection(cursor..cursor);
    let context = Context::default();
    let frame = hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
    let text = frame
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.starts_with('☐') => Some(text),
            _ => None,
        })
        .expect("active task list must be rendered");
    let position = text.pos
        + text
            .galley
            .pos_from_cursor(CCursor::new(0))
            .center()
            .to_vec2()
        + egui::vec2(3.0, 0.0);
    for pressed in [true, false] {
        hybrid_test_frame(
            &mut app,
            &context,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            egui::Modifiers::NONE,
        );
    }
    assert_eq!(app.session[0].content, "- [x] 第一项\n- [ ] 第二项");
    app.undo_active();
    assert_eq!(app.session[0].content, source);
}

#[test]
fn fenced_code_arrow_navigation_never_inserts_lines_into_the_document() {
    for (key, tail) in [
        (Key::ArrowDown, "\n\nfollowing"),
        (Key::ArrowDown, "\nfollowing"),
        (Key::ArrowDown, "\n\n\n\nfollowing"),
        (Key::ArrowDown, ""),
        (Key::ArrowRight, "\n\nfollowing"),
        (Key::ArrowRight, ""),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        let source = format!("```\nfirst\nlast\n```{tail}");
        app.session[0].content = source.clone();
        app.session[0].update_after_edit();
        let code_end = "```\nfirst\nlast".chars().count();
        app.queue_editor_selection(code_end..code_end);
        let context = Context::default();
        hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
        for _ in 0..2 {
            hybrid_test_frame(
                &mut app,
                &context,
                vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
            assert_eq!(app.session[0].content, source, "tail={tail:?}");
        }
        if !tail.is_empty() {
            assert!(
                app.editor_surface.cursor().unwrap().primary.index.0
                    >= source.find("following").unwrap(),
                "the arrow should move into the following paragraph: key={key:?}, tail={tail:?}, cursor={:?}",
                app.editor_surface.cursor()
            );
        }
    }
}

#[test]
fn wysiwyg_backspace_removes_extra_blank_lines_after_code_without_changing_code() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    let code = "```\nfirst\nlast\n```";
    app.session[0].content = format!("{code}\n\n\n\n正文");
    app.session[0].update_after_edit();
    let cursor = code.chars().count() + 4;
    app.queue_editor_selection(cursor..cursor);
    let context = Context::default();
    hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
    for gap in ["\n\n", "\n"] {
        hybrid_test_frame(
            &mut app,
            &context,
            vec![egui::Event::Key {
                key: Key::Backspace,
                physical_key: Some(Key::Backspace),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.session[0].content, format!("{code}{gap}正文"));
    }
}

#[test]
fn fenced_code_enter_preserves_literal_text_without_markdown_continuations() {
    for (body, modifiers) in [
        ("- item", egui::Modifiers::NONE),
        ("1. item", egui::Modifiers::NONE),
        ("> item", egui::Modifiers::NONE),
        ("value", egui::Modifiers::SHIFT),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        let source = format!("```text\n{body}\n```");
        app.session[0].content = source.clone();
        app.session[0].update_after_edit();
        let cursor = "```text\n".chars().count() + body.chars().count();
        app.queue_editor_selection(cursor..cursor);
        let context = Context::default();
        hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
        hybrid_test_frame(
            &mut app,
            &context,
            vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers,
            }],
            modifiers,
        );
        assert_eq!(
            app.session[0].content,
            format!("```text\n{body}\n\n```"),
            "body={body:?}, modifiers={modifiers:?}"
        );
        app.undo_active();
        assert_eq!(app.session[0].content, source);
    }
}

#[test]
fn fenced_code_paste_replaces_selection_with_literal_url() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    let source = "```text\nreplace me\n```";
    app.session[0].content = source.to_owned();
    app.session[0].update_after_edit();
    let start = "```text\n".chars().count();
    app.queue_editor_selection(start..start + "replace me".len());
    let context = Context::default();
    hybrid_test_frame(&mut app, &context, Vec::new(), egui::Modifiers::NONE);
    hybrid_test_frame(
        &mut app,
        &context,
        vec![egui::Event::Paste("https://example.com/path".to_owned())],
        egui::Modifiers::NONE,
    );
    assert_eq!(
        app.session[0].content,
        "```text\nhttps://example.com/path\n```"
    );
}

#[test]
fn fenced_code_surface_never_overlaps_the_following_paragraph() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].content = "```\nddwadada\nadawd\n```\n\nwdadawd".to_owned();
    app.session[0].update_after_edit();
    let context = Context::default();
    for editing in [false, true, false] {
        if editing {
            app.queue_editor_selection(7..7);
        } else {
            app.editor_surface.invalidate_content();
        }
        let (code, paragraph) = code_and_following_paragraph_rects(&mut app, &context);
        assert!(
            paragraph.top() >= code.bottom(),
            "editing={editing}: code={code:?}, paragraph={paragraph:?}"
        );
    }
}

#[test]
fn fenced_code_keeps_its_size_when_switching_between_preview_and_editing() {
    for (body, separator) in [
        ("", "\n\n"),
        ("line", "\n\n"),
        ("ddwadada\nadawd", "\n\n"),
        ("line\n\n", "\n\n"),
        ("line", "\n\n\n\n"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.session[0].content = format!("```\n{body}\n```{separator}wdadawd");
        app.session[0].update_after_edit();
        let context = Context::default();
        let (preview, paragraph_before) = code_and_following_paragraph_rects(&mut app, &context);
        app.queue_editor_selection(4..4);
        let (editor, paragraph_after) = code_and_following_paragraph_rects(&mut app, &context);
        assert!(
            (preview.height() - editor.height()).abs() <= 1.0,
            "body={body:?}: preview={preview:?}, editor={editor:?}"
        );
        assert!(
            (preview.width() - editor.width()).abs() <= 1.0,
            "code width must remain stable"
        );
        assert!(
            (preview.top() - editor.top()).abs() <= 1.0,
            "code position must remain stable"
        );
        assert!(
            (paragraph_before.top() - paragraph_after.top()).abs() <= 1.0,
            "the following paragraph must not jump: before={paragraph_before:?}, after={paragraph_after:?}"
        );
    }
}

#[test]
fn audit_cross_block_input_preserves_all_text_events_and_normalizes_pasted_newlines() {
    for (events, expected) in [
        (
            vec![
                egui::Event::Text("你".to_owned()),
                egui::Event::Text("🙂".to_owned()),
            ],
            "你🙂",
        ),
        (
            vec![egui::Event::Paste("甲\r\n乙\r丙".to_owned())],
            "甲\n乙\n丙",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.session[0].content = "first\n\nsecond".to_owned();
        app.session[0].update_after_edit();
        let cursor = CCursorRange::two(CCursor::new(0), CCursor::new(13));
        app.editor_surface.select_cursor(cursor);
        let context = Context::default();
        let _ = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.hybrid_pane(ui, 0),
        );
        assert_eq!(app.session[0].content, expected);
        app.undo_active();
        assert_eq!(app.session[0].content, "first\n\nsecond");
    }
}

#[test]
fn audit_source_editor_pastes_plain_text_over_a_unicode_selection() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].content = "前中文🙂后".to_owned();
    app.session[0].update_after_edit();
    app.queue_editor_selection(1..4);
    let context = Context::default();
    for events in [vec![], vec![egui::Event::Paste("替换".to_owned())]] {
        let _ = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                app.edit_pane(ui, 0, None);
            },
        );
    }
    assert_eq!(app.session[0].content, "前替换后");
    app.undo_active();
    assert_eq!(app.session[0].content, "前中文🙂后");
}

#[test]
fn audit_switching_untitled_tabs_restores_each_selection() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].content = "甲乙🙂".to_owned();
    app.queue_editor_selection(1..3);
    app.new_document();
    app.session[1].content = "第二篇".to_owned();
    app.queue_editor_selection(0..2);
    app.activate_document(0);
    assert_eq!(app.active_selection(0), 1..3);
    app.activate_document(1);
    assert_eq!(app.active_selection(1), 0..2);
}

#[test]
fn audit_closing_an_inactive_tab_preserves_the_active_selection() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.new_document();
    app.session[1].content = "正文🙂".to_owned();
    app.queue_editor_selection(0..2);
    app.close_document(0);
    assert_eq!(app.session.active_index(), Some(0));
    assert_eq!(app.active_selection(0), 0..2);
}

#[test]
fn audit_reloading_multiple_files_resets_the_active_editor_even_when_it_is_not_last() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    for name in ["first.md", "second.md"] {
        let path = directory.path().join(name);
        fs::write(&path, "original").unwrap();
        app.session.insert(Document::open(&path).unwrap());
        fs::write(path, "changed 中文🙂").unwrap();
    }
    app.session.activate(app.session[0].id());
    app.queue_editor_selection(0..8);
    app.external_changes = ExternalChanges::new(Instant::now() - Duration::from_secs(3));
    app.check_external_changes_if_due();
    assert_eq!(app.session[0].content, "changed 中文🙂");
    assert!(app.editor_surface.cursor().is_none());
    assert!(app.editor_surface.bookmark().cursor.is_none());
}

#[test]
fn accepts_supported_document_extensions_case_insensitively() {
    assert!(is_markdown_path(Path::new("README.MD")));
    assert!(is_markdown_path(Path::new("notes.markdown")));
    assert!(!is_markdown_path(Path::new("image.png")));
}

#[test]
fn decodes_unicode_local_link_fragments_without_form_semantics() {
    assert_eq!(
        decode_uri_fragment("%E4%B8%AD%E6%96%87%20%E6%A0%87%E9%A2%98"),
        Some("中文 标题".to_owned())
    );
    assert_eq!(decode_uri_fragment("c%2B%2B"), Some("c++".to_owned()));
    assert_eq!(decode_uri_fragment("a+b"), Some("a+b".to_owned()));
    assert_eq!(decode_uri_fragment("%ZZ"), None);
    assert_eq!(decode_uri_fragment("%E4%B8"), None);
}

#[test]
fn extension_results_follow_document_identity_after_tabs_shift() {
    let first = Document::untitled(1);
    let target = Document::untitled(2);
    let target_id = target.id();
    let trailing = Document::untitled(3);
    let first_id = first.id();
    let mut session = DocumentSession::default();
    session.insert(first);
    session.insert(target);
    session.insert(trailing);
    session.remove(first_id);
    assert_eq!(session.index_of(target_id), Some(0));
    session.remove(target_id);
    assert_eq!(session.index_of(target_id), None);
}

#[test]
fn clean_session_documents_are_restored_alongside_recovered_tabs() {
    let directory = tempfile::tempdir().unwrap();
    let recovered_path = directory.path().join("recovered.md");
    let clean_path = directory.path().join("clean.md");
    fs::write(&recovered_path, "recovered").unwrap();
    fs::write(&clean_path, "clean").unwrap();
    let recovered = Document::open(&recovered_path).unwrap();

    let mut session = DocumentSession::default();
    session.insert(recovered);
    let paths = session.restorable_files([recovered_path.clone(), clean_path.clone()]);
    assert_eq!(paths, vec![clean_path]);
}

#[test]
fn allocates_the_first_unused_numeric_footnote() {
    assert_eq!(next_footnote_number("plain"), 1);
    assert_eq!(next_footnote_number("[^1] and [^3]"), 2);
    assert_eq!(next_footnote_number("[^2]: definition"), 1);
    assert_eq!(next_footnote_number("[^1][^2]"), 3);
    assert_eq!(next_footnote_number("[^[^1][^2]"), 3);
}

#[test]
fn creates_relative_encoded_resource_destinations() {
    let directory = tempfile::tempdir().unwrap();
    let notes = directory.path().join("notes");
    let assets = directory.path().join("assets");
    fs::create_dir_all(&notes).unwrap();
    fs::create_dir_all(&assets).unwrap();
    let image = assets.join("diagram one.png");
    fs::write(&image, b"image").unwrap();

    assert_eq!(
        markdown_resource_destination(&image, &notes),
        "../assets/diagram%20one.png"
    );
    assert!(is_image_path(&image));
    assert!(!is_image_path(Path::new("attachment.pdf")));
}

#[test]
fn parses_configurable_cross_platform_shortcuts() {
    let shortcut = parse_shortcut("Ctrl+Shift+P").unwrap();
    assert!(shortcut.modifiers.command);
    assert!(shortcut.modifiers.shift);
    assert_eq!(shortcut.logical_key, Key::P);
    assert!(parse_shortcut("Ctrl+NoSuchKey").is_none());
}

#[test]
fn detects_duplicate_shortcuts() {
    let mut bindings = KeyBindings::default();
    assert!(!duplicate_shortcuts(&bindings));
    bindings.link.clone_from(&bindings.bold);
    assert!(duplicate_shortcuts(&bindings));
}

#[test]
fn local_path_policy_blocks_workspace_escape() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    let outside = directory.path().join("outside.txt");
    fs::create_dir(&workspace).unwrap();
    fs::write(&outside, "secret").unwrap();
    let inside = workspace.join("note.md");
    fs::write(&inside, "safe").unwrap();

    assert!(path_is_within(&inside, &workspace));
    assert!(!path_is_within(&outside, &workspace));
}

#[test]
fn heading_line_navigation_handles_unicode_and_out_of_range_lines() {
    let source = "标题\r\n第二行\nthird";
    assert_eq!(line_start_byte(source, 1), 0);
    assert_eq!(&source[line_start_byte(source, 2)..], "第二行\nthird");
    assert_eq!(&source[line_start_byte(source, 3)..], "third");
    assert_eq!(line_start_byte(source, 99), source.len());
}
