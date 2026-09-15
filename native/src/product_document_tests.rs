use super::*;

#[test]
fn product_document_reference_context_updates_and_preserves_link_positions() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = product_app(directory.path());
    app.new_document();
    let block = "中文 [Link][STRASSE] and ![Picture][pic].";
    for destination in ["#first", "#second"] {
        app.session[0].content = format!("{block}\n\n[straße]: {destination}\n[pic]: image.png\n");
        app.session[0].update_after_edit();
        let references = app.session[0].references();
        let projection =
            VisualProjection::from_markdown_with_context(block, None, references.clone());
        assert!(!projection.text().contains("[STRASSE]"));
        let offset = projection.text().find("Link").unwrap();
        let char_offset = projection.text()[..offset].chars().count();
        let local = projection
            .source_char_range(block, char_offset..char_offset)
            .start;
        assert_eq!(
            markdown::link_destination_with_references(
                block,
                char_to_byte(block, local),
                &references
            )
            .as_deref(),
            Some(destination)
        );
        assert!(Arc::ptr_eq(&references, &app.session[0].references()));
    }
}

fn product_app(directory: &Path) -> RuporaApp {
    RuporaApp::from_state(
        PersistedState::default(),
        RecoveryStore::at(directory.join("recovery.json")),
        None,
        ExtensionRegistry::disabled(directory.join("extensions.json")),
        None,
    )
}

fn product_frame(
    app: &mut RuporaApp,
    context: &Context,
    hybrid: bool,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let index = app.session.active_index().unwrap();
    context.run_ui(
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
                app.hybrid_pane(ui, index);
            } else {
                app.edit_pane(ui, index, None);
            }
        },
    )
}

fn product_painted_text(output: &egui::FullOutput) -> String {
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
fn product_document_indented_code_stays_literal_in_both_reading_canvases() {
    for hybrid in [false, true] {
        for active_code in [false, true] {
            for indent in ["    ", "\t"] {
                let directory = tempfile::tempdir().unwrap();
                let mut app = product_app(directory.path());
                app.new_document();
                let source = format!("前文\n\n{indent}**中文🙂**\n{indent}[x](secret.md)\n\n后文");
                app.session[0].content = source.clone();
                app.session[0].update_after_edit();
                if active_code {
                    let caret = source[..source.find("中文").unwrap()].chars().count();
                    app.queue_editor_selection(caret..caret);
                }
                let context = Context::default();
                install_fonts(&context);
                let output = context.run_ui(egui::RawInput::default(), |ui| {
                    if hybrid {
                        app.hybrid_pane(ui, 0);
                    } else {
                        app.preview_pane(ui, 0, None);
                    }
                });
                let painted = product_painted_text(&output);
                assert!(
                    painted.contains("**中文🙂**"),
                    "hybrid={hybrid}, active={active_code}: {painted:?}"
                );
                assert!(
                    painted.contains("[x](secret.md)"),
                    "hybrid={hybrid}, active={active_code}: {painted:?}"
                );
            }
        }
    }
}

#[test]
fn product_document_indented_code_unicode_edit_and_undo_preserve_source() {
    for indent in ["    ", "\t"] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = product_app(directory.path());
        app.new_document();
        let source = format!("前文\n\n{indent}**中文🙂**\n{indent}[x](secret.md)\n\n后文");
        app.session[0].content = source.clone();
        app.session[0].update_after_edit();
        let start = source[..source.find("中文🙂").unwrap()].chars().count();
        app.queue_editor_selection(start..start + 3);
        let context = Context::default();
        install_fonts(&context);
        product_frame(&mut app, &context, true, vec![]);
        product_frame(
            &mut app,
            &context,
            true,
            vec![egui::Event::Text("替换😀".to_owned())],
        );
        assert_eq!(app.session[0].content, source.replace("中文🙂", "替换😀"));
        app.undo_active();
        assert_eq!(app.session[0].content, source);
        app.redo_active();
        assert_eq!(app.session[0].content, source.replace("中文🙂", "替换😀"));
    }
}

#[test]
fn product_document_tab_selection_undo_redo_and_save_are_isolated() {
    for hybrid in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = product_app(directory.path());
        app.new_document();
        let source = "甲乙\n\n丙丁\n\n末尾";
        app.session[0].content = source.to_owned();
        app.session[0].update_after_edit();
        app.session[0]
            .save_as(directory.path().join("a.md"), false)
            .unwrap();
        app.queue_editor_selection(1..7);
        let context = Context::default();
        product_frame(&mut app, &context, hybrid, vec![]);
        app.new_document();
        app.session[1].content = "SECOND".to_owned();
        app.session[1].update_after_edit();
        app.queue_editor_selection(2..4);
        product_frame(&mut app, &context, hybrid, vec![]);
        app.activate_document(0);
        assert_eq!(app.active_selection(0), 1..7);
        product_frame(&mut app, &context, hybrid, vec![]);
        product_frame(
            &mut app,
            &context,
            hybrid,
            vec![egui::Event::Text("替换".to_owned())],
        );
        assert_eq!(app.session[0].content, "甲替换\n末尾", "hybrid={hybrid}");
        app.undo_active();
        assert_eq!(app.session[0].content, source);
        assert_eq!(app.active_selection(0), 1..7);
        app.redo_active();
        assert_eq!(app.session[0].content, "甲替换\n末尾");
        app.save_active(false);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("a.md")).unwrap(),
            app.session[0].content
        );
        assert!(!app.session[0].dirty);
        app.activate_document(1);
        assert_eq!(app.session[1].content, "SECOND");
        assert_eq!(app.active_selection(1), 2..4);
    }
}

#[test]
fn product_document_reference_links_render_in_both_reading_canvases() {
    let mut failures = Vec::new();
    for hybrid in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = product_app(directory.path());
        app.new_document();
        let source = "See [Documentation][docs].\n\nOther paragraph.\n\n[docs]: https://example.com/docs \"Docs title\"\n";
        assert!(
            markdown::render_html_fragment(source).contains("href=\"https://example.com/docs\"")
        );
        app.session[0].content = source.to_owned();
        app.session[0].update_after_edit();
        let context = Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            if hybrid {
                app.hybrid_pane(ui, 0);
            } else {
                app.preview_pane(ui, 0, None);
            }
        });
        let painted = product_painted_text(&output);
        if painted.contains("[Documentation][docs]") {
            failures.push(format!("hybrid={hybrid}: {painted:?}"));
        }
        assert!(painted.contains("Documentation"));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_document_table_new_cell_pipe_backslash_roundtrip() {
    let mut failures = Vec::new();
    let mut cells = vec![
        r"\*literal\*".to_owned(),
        r"`a|b`".to_owned(),
        "中文😀|e\u{301}".to_owned(),
    ];
    for count in 0..=5 {
        cells.push(format!("x{}|y", "\\".repeat(count)));
        cells.push(format!("x{}", "\\".repeat(count)));
    }
    for cell in cells {
        let mut table = table::new_table(0);
        table.rows = vec![vec![cell.clone(), "KEEP".to_owned()]];
        let serialized = table.to_markdown();
        let parsed = table::find_table(&serialized, 0).unwrap();
        let first_same_semantics = markdown::render_html_fragment(&parsed.rows[0][0])
            == markdown::render_html_fragment(&cell);
        if parsed.rows[0][1] != "KEEP" || !first_same_semantics {
            failures.push(format!(
                "cell={cell:?}, actual={:?}, serialized={serialized:?}",
                parsed.rows
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn product_document_table_remove_last_row_and_column_save_roundtrip() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = product_app(directory.path());
    app.new_document();
    let source = "BEFORE\n\n| A | B |\n| --- | --- |\n| x | y |\n\nAFTER";
    app.session[0].content = source.to_owned();
    app.session[0].update_after_edit();
    app.session[0]
        .save_as(directory.path().join("table.md"), false)
        .unwrap();
    app.queue_editor_selection(25..25);
    app.open_table_editor();
    let table = &mut app.table_editor.as_mut().unwrap().table;
    table.remove_row();
    table.remove_row();
    table.remove_column();
    table.remove_column();
    app.apply_table_editor();
    let expected = "BEFORE\n\n| A   |\n| --- |\n\nAFTER";
    assert_eq!(app.session[0].content, expected);
    app.save_active(false);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("table.md")).unwrap(),
        expected
    );
    app.undo_active();
    assert_eq!(app.session[0].content, source);
    assert!(app.session[0].dirty);
}

#[test]
fn product_document_recovery_merges_external_edits_and_keeps_conflicts() {
    for conflicting in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery-target.md");
        std::fs::write(&path, "one\nmiddle\nthree\n").unwrap();
        let store = RecoveryStore::at(directory.path().join("recovery.json"));
        let mut document = Document::open(&path).unwrap();
        document.content = "LOCAL\nmiddle\nthree\n".to_owned();
        document.update_after_edit();
        store.save(&[document]).unwrap();
        let external = if conflicting {
            "EXTERNAL\nmiddle\nthree\n"
        } else {
            "one\nmiddle\nEXTERNAL\n"
        };
        std::fs::write(&path, external).unwrap();
        let entry = store.load().unwrap().pop().unwrap();
        let outcome = Document::recover(
            entry.path,
            entry.content,
            entry.base_content,
            entry.encoding.as_deref(),
            entry.line_ending.as_deref(),
            1,
        );
        let mut recovered = outcome.document;
        assert_eq!(outcome.conflicts, usize::from(conflicting));
        assert!(recovered.content.contains("LOCAL"));
        assert!(recovered.content.contains("EXTERNAL"));
        if !conflicting {
            assert_eq!(recovered.content, "LOCAL\nmiddle\nEXTERNAL\n");
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
        recovered.save(false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), recovered.content);
    }
}

#[test]
fn product_document_clean_reload_and_dirty_external_guard() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.md");
    let b = directory.path().join("b.md");
    std::fs::write(&a, "ORIGINAL A").unwrap();
    std::fs::write(&b, "ORIGINAL B").unwrap();
    let mut app = product_app(directory.path());
    app.open_paths(vec![a.clone(), b.clone()]);
    app.session[1].content = "LOCAL B".to_owned();
    app.session[1].update_after_edit();
    std::fs::write(&a, "EXTERNAL A LONGER").unwrap();
    std::fs::write(&b, "EXTERNAL B LONGER").unwrap();
    app.external_changes = ExternalChanges::new(Instant::now() - Duration::from_secs(3));
    app.check_external_changes_if_due();
    assert_eq!(app.session[0].content, "EXTERNAL A LONGER");
    assert!(!app.session[0].dirty);
    assert_eq!(app.session[1].content, "LOCAL B");
    assert!(app.external_changes.has_conflict(app.session[1].id()));
    assert!(app.session[1].save(false).is_err());
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "EXTERNAL B LONGER");
}

#[test]
fn product_document_reference_image_renders_in_both_reading_canvases() {
    let mut failures = Vec::new();
    for hybrid in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        image::save_buffer_with_format(
            directory.path().join("pixel.png"),
            &[255, 0, 0, 255],
            1,
            1,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        let mut app = product_app(directory.path());
        app.new_document();
        let source = "![Diagram][img]\n\nSome paragraph.\n\n[img]: pixel.png\n";
        assert!(markdown::render_html_fragment(source).contains("<img"));
        app.session[0].content = source.to_owned();
        app.session[0].update_after_edit();
        app.session[0]
            .save_as(directory.path().join("image.md"), false)
            .unwrap();
        let context = Context::default();
        egui_extras::install_image_loaders(&context);
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            if hybrid {
                app.hybrid_pane(ui, 0);
            } else {
                app.preview_pane(ui, 0, None);
            }
        });
        let painted = product_painted_text(&output);
        assert!(painted.contains("Diagram"));
        if painted.contains("![Diagram][img]") {
            failures.push(format!("hybrid={hybrid}: {painted:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn product_document_footnote_cross_block_text_renders_in_both_reading_canvases() {
    for hybrid in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = product_app(directory.path());
        app.new_document();
        app.session[0].content =
            "Sentence[^note].\n\nOther paragraph.\n\n[^note]: Footnote body.\n".to_owned();
        app.session[0].update_after_edit();
        let context = Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            if hybrid {
                app.hybrid_pane(ui, 0);
            } else {
                app.preview_pane(ui, 0, None);
            }
        });
        let painted = product_painted_text(&output);
        assert!(
            painted.contains("Footnote body."),
            "hybrid={hybrid}: {painted:?}"
        );
        assert!(!painted.contains("[^note]"), "hybrid={hybrid}: {painted:?}");
    }
}

#[test]
fn product_document_reference_link_stays_resolved_while_editing_surrounding_text() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = product_app(directory.path());
    app.new_document();
    let source =
        "See [Documentation][docs].\n\nOther paragraph.\n\n[docs]: https://example.com/docs\n";
    app.session[0].content = source.to_owned();
    app.session[0].update_after_edit();
    app.queue_editor_selection(0..0);
    let context = Context::default();
    let output = product_frame(&mut app, &context, true, vec![]);
    let painted = product_painted_text(&output);
    assert!(!painted.contains("[Documentation][docs]"), "{painted:?}");
    product_frame(
        &mut app,
        &context,
        true,
        vec![egui::Event::Text("Please ".to_owned())],
    );
    assert_eq!(app.session[0].content, format!("Please {source}"));
}
