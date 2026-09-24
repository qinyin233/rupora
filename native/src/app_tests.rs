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

fn shortcut_editor_frame(
    app: &mut RuporaApp,
    ctx: &Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 800.0),
        )),
        events,
        ..Default::default()
    };
    eframe::App::raw_input_hook(app, ctx, &mut raw);
    ctx.run_ui(raw, |ui| {
        let mut frame = Frame::_new_kittest();
        eframe::App::logic(app, ctx, &mut frame);
        eframe::App::ui(app, ui, &mut frame);
    })
}

#[test]
fn ime_batches_keep_prefixes_and_candidates_across_pass_limits() {
    for mode in [ViewMode::Edit, ViewMode::Split, ViewMode::Hybrid] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.state.view_mode = mode;
        app.session[0].content = "A🙂B".into();
        app.session[0].update_after_edit();
        app.queue_editor_selection(1..1);
        let ctx = Context::default();
        install_fonts(&ctx);
        shortcut_editor_frame(&mut app, &ctx, vec![]);
        let events = (0..12)
            .flat_map(|_| {
                [
                    egui::Event::Text("X".into()),
                    egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: "ni".into(),
                        active_range_chars: Some(0..2),
                    }),
                    egui::Event::Ime(egui::ImeEvent::Commit("你".into())),
                ]
            })
            .collect();
        shortcut_editor_frame(&mut app, &ctx, events);
        assert!(app.ordered_input_pass.is_some());
        shortcut_editor_frame(&mut app, &ctx, vec![egui::Event::Text("尾".into())]);
        drain_ordered_input(&mut app, &ctx);
        let expected = format!("A{}尾🙂B", "X你".repeat(12));
        assert_eq!(app.session[0].content, expected, "{mode:?}");
        while app.session[0].can_undo() {
            app.undo_active();
        }
        assert_eq!(app.session[0].content, "A🙂B");
        while app.session[0].can_redo() {
            app.redo_active();
        }
        assert_eq!(app.session[0].content, expected);
    }
}

#[test]
fn ime_commit_then_preedit_and_native_focus_loss_keeps_only_candidate() {
    for mode in [ViewMode::Edit, ViewMode::Split] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.state.view_mode = mode;
        app.session[0].content = "A🙂B".into();
        app.session[0].update_after_edit();
        app.queue_editor_selection(1..1);
        let ctx = Context::default();
        install_fonts(&ctx);
        shortcut_editor_frame(&mut app, &ctx, vec![]);
        shortcut_raw_frame(
            &mut app,
            &ctx,
            egui::RawInput {
                focused: false,
                events: vec![
                    egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: "zhong".into(),
                        active_range_chars: Some(0..5),
                    }),
                    egui::Event::Ime(egui::ImeEvent::Commit("中".into())),
                    egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: "wen".into(),
                        active_range_chars: Some(0..3),
                    }),
                    egui::Event::WindowFocused(false),
                ],
                ..Default::default()
            },
            true,
        );
        drain_ordered_input(&mut app, &ctx);
        assert_eq!(app.session[0].content, "A中🙂B", "{mode:?}");
        shortcut_editor_frame(&mut app, &ctx, vec![egui::Event::Text("末".into())]);
        assert_eq!(app.session[0].content, "A中末🙂B", "{mode:?}");
        while app.session[0].can_undo() {
            app.undo_active();
        }
        assert_eq!(app.session[0].content, "A🙂B");
    }
}

#[test]
fn ime_with_scroll_or_held_pointer_preserves_plain_input_before_cancellation() {
    for held_pointer in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.state.view_mode = ViewMode::Edit;
        app.session[0].content = "A🙂B".into();
        app.session[0].update_after_edit();
        app.queue_editor_selection(1..1);
        let ctx = Context::default();
        install_fonts(&ctx);
        shortcut_editor_frame(&mut app, &ctx, vec![]);
        if held_pointer {
            shortcut_editor_frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerButton {
                    pos: egui::pos2(990.0, 790.0),
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            app.queue_editor_selection(1..1);
            shortcut_editor_frame(&mut app, &ctx, vec![]);
        }
        let mut events = vec![
            egui::Event::Text("X".into()),
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "ni".into(),
                active_range_chars: Some(0..2),
            }),
            egui::Event::Ime(egui::ImeEvent::Commit(String::new())),
        ];
        if !held_pointer {
            events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -5.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
        }
        shortcut_editor_frame(&mut app, &ctx, events);
        drain_ordered_input(&mut app, &ctx);
        assert_eq!(
            app.session[0].content, "AX🙂B",
            "held_pointer={held_pointer}"
        );
        app.undo_active();
        assert_eq!(app.session[0].content, "A🙂B");
    }
}

#[test]
fn ime_and_close_deliver_candidate_before_the_single_close_request() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.state.view_mode = ViewMode::Edit;
    app.queue_editor_selection(0..0);
    let ctx = Context::default();
    shortcut_editor_frame(&mut app, &ctx, vec![]);
    // Avoid opening a native confirmation dialog in this scheduler regression.
    app.allow_close = true;
    let mut raw = egui::RawInput {
        events: vec![
            egui::Event::Ime(egui::ImeEvent::Commit("中".into())),
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "wen".into(),
                active_range_chars: Some(0..3),
            }),
        ],
        ..Default::default()
    };
    raw.viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let output = shortcut_raw_frame(&mut app, &ctx, raw, true);
    assert!(
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&ViewportCommand::CancelClose)
    );
    assert_eq!(app.session[0].content, "中");
    let mut closes = 0;
    for _ in 0..8 {
        let output = shortcut_editor_frame(&mut app, &ctx, vec![]);
        closes += output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .filter(|command| matches!(command, ViewportCommand::Close))
            .count();
    }
    assert_eq!(closes, 1);
    assert_eq!(app.session[0].content, "中");
}

#[test]
fn ime_interruption_reaches_the_backend_after_all_input_passes() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.state.view_mode = ViewMode::Edit;
    app.session[0].content = "A🙂B".into();
    app.session[0].update_after_edit();
    app.queue_editor_selection(1..1);
    let ctx = Context::default();
    shortcut_editor_frame(&mut app, &ctx, vec![]);
    shortcut_editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "ni".into(),
            active_range_chars: Some(0..2),
        })],
    );
    let output = shortcut_editor_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Text("X".into()),
            egui::Event::Ime(egui::ImeEvent::Commit(String::new())),
        ],
    );
    assert_eq!(app.session[0].content, "AX🙂B");
    assert!(
        output
            .platform_output
            .ime
            .unwrap()
            .should_interrupt_composition
    );
}

#[test]
fn ime_new_preedit_after_literal_input_supersedes_the_old_interruption() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.state.view_mode = ViewMode::Edit;
    app.session[0].content = "A🙂B".into();
    app.session[0].update_after_edit();
    app.queue_editor_selection(1..1);
    let ctx = Context::default();
    shortcut_editor_frame(&mut app, &ctx, vec![]);
    let preedit = egui::Event::Ime(egui::ImeEvent::Preedit {
        text: "ni".into(),
        active_range_chars: Some(0..2),
    });
    shortcut_editor_frame(&mut app, &ctx, vec![preedit.clone()]);
    let output =
        shortcut_editor_frame(&mut app, &ctx, vec![egui::Event::Text("X".into()), preedit]);
    assert_eq!(app.session[0].content, "AX🙂B");
    assert!(
        !output
            .platform_output
            .ime
            .unwrap()
            .should_interrupt_composition
    );
    shortcut_editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new()))],
    );
    assert_eq!(app.session[0].content, "AX🙂B");
}

fn command_key(key: Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: !cfg!(target_os = "macos"),
            mac_cmd: cfg!(target_os = "macos"),
            command: true,
            ..Default::default()
        },
    }
}

#[test]
fn ime_late_preedit_after_window_focus_loss_never_changes_the_document() {
    for mode in [ViewMode::Edit, ViewMode::Split, ViewMode::Hybrid] {
        for explicit_focus_event in [false, true] {
            for already_composing in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let mut app = isolated_app(directory.path());
                app.new_document();
                app.state.view_mode = mode;
                app.session[0].content = "A🙂B".into();
                app.session[0].update_after_edit();
                app.queue_editor_selection(1..1);
                let ctx = Context::default();
                install_fonts(&ctx);
                shortcut_editor_frame(&mut app, &ctx, vec![]);
                let token = app.session[0].snapshot_token();
                if already_composing {
                    shortcut_editor_frame(
                        &mut app,
                        &ctx,
                        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                            text: "zhong'wen".into(),
                            active_range_chars: Some(0..8),
                        })],
                    );
                }
                shortcut_raw_frame(
                    &mut app,
                    &ctx,
                    egui::RawInput {
                        focused: false,
                        events: if explicit_focus_event {
                            vec![egui::Event::WindowFocused(false)]
                        } else {
                            vec![]
                        },
                        ..Default::default()
                    },
                    true,
                );
                shortcut_raw_frame(
                    &mut app,
                    &ctx,
                    egui::RawInput {
                        focused: false,
                        events: vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                            text: "ni".into(),
                            active_range_chars: Some(0..2),
                        })],
                        ..Default::default()
                    },
                    true,
                );
                assert_eq!(
                    app.session[0].content, "A🙂B",
                    "mode={mode:?}, explicit_focus_event={explicit_focus_event}, already_composing={already_composing}"
                );
                assert_eq!(app.session[0].snapshot_token(), token);
                assert!(!app.session[0].can_undo());
                shortcut_editor_frame(
                    &mut app,
                    &ctx,
                    vec![
                        egui::Event::WindowFocused(true),
                        egui::Event::Ime(egui::ImeEvent::Commit(String::new())),
                        egui::Event::Text("X".into()),
                    ],
                );
                drain_ordered_input(&mut app, &ctx);
                assert_eq!(
                    app.session[0].content, "AX🙂B",
                    "{mode:?}, explicit_focus_event={explicit_focus_event}"
                );
                app.undo_active();
                assert_eq!(app.session[0].content, "A🙂B");
            }
        }
    }
}

#[test]
fn multiple_format_shortcuts_match_separate_frames() {
    let events = vec![
        egui::Event::Text("中".into()),
        command_key(Key::B),
        egui::Event::Text("文".into()),
        command_key(Key::I),
        egui::Event::Text("尾".into()),
    ];
    let mut outcomes = Vec::new();
    for batched in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.state.view_mode = ViewMode::Edit;
        app.session[0].content = "AB".into();
        app.session[0].update_after_edit();
        app.queue_editor_selection(1..1);
        let ctx = Context::default();
        install_fonts(&ctx);
        shortcut_editor_frame(&mut app, &ctx, vec![]);
        if batched {
            shortcut_editor_frame(&mut app, &ctx, events.clone());
        } else {
            for event in events.clone() {
                shortcut_editor_frame(&mut app, &ctx, vec![event]);
            }
        }
        outcomes.push((app.session[0].content.clone(), app.active_selection(0)));
    }
    assert_eq!(
        outcomes[1], outcomes[0],
        "batching must preserve command order"
    );
}

fn drain_ordered_input(app: &mut RuporaApp, ctx: &Context) {
    for _ in 0..64 {
        if app.ordered_input_pass.is_none()
            && !app.ordered_shell_pending
            && app.ordered_raw_input.is_empty()
        {
            return;
        }
        shortcut_editor_frame(app, ctx, vec![]);
    }
    panic!("ordered input did not drain");
}

type ShortcutSnapshot = (String, std::ops::Range<usize>, Option<CCursorRange>);

fn shortcut_snapshot(app: &RuporaApp) -> ShortcutSnapshot {
    (
        app.session[0].content.clone(),
        app.active_selection(0),
        app.editor_surface.bookmark().cursor,
    )
}

fn shortcut_history(app: &mut RuporaApp) -> Vec<ShortcutSnapshot> {
    let mut history = vec![shortcut_snapshot(app)];
    while app.session[0].can_undo() {
        app.undo_active();
        history.push(shortcut_snapshot(app));
        assert!(history.len() < 100);
    }
    while app.session[0].can_redo() {
        app.redo_active();
        history.push(shortcut_snapshot(app));
        assert!(history.len() < 200);
    }
    history
}

#[test]
fn ordered_shortcuts_match_native_individual_commands_and_history() {
    let right = egui::Event::Key {
        key: Key::ArrowRight,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let scenarios = vec![
        vec![command_key(Key::B), command_key(Key::K)],
        vec![command_key(Key::K), command_key(Key::B)],
        vec![command_key(Key::B), command_key(Key::B)],
        vec![
            command_key(Key::B),
            command_key(Key::K),
            egui::Event::Text("中".into()),
        ],
        vec![
            egui::Event::Text("中".into()),
            command_key(Key::B),
            egui::Event::Text("文".into()),
            command_key(Key::Z),
        ],
        vec![
            command_key(Key::B),
            command_key(Key::Z),
            command_key(Key::Y),
        ],
        vec![
            right.clone(),
            command_key(Key::B),
            egui::Event::Text("中".into()),
            command_key(Key::I),
            egui::Event::Text("文".into()),
        ],
        vec![
            egui::Event::Text("中".into()),
            command_key(Key::B),
            egui::Event::Text("文".into()),
            egui::Event::PointerMoved(egui::pos2(900.0, 700.0)),
        ],
        vec![
            egui::Event::Ime(egui::ImeEvent::Commit("中".into())),
            command_key(Key::B),
            egui::Event::Ime(egui::ImeEvent::Commit("文".into())),
            command_key(Key::I),
            egui::Event::Text("尾".into()),
        ],
        vec![
            right,
            egui::Event::Key {
                key: Key::ArrowLeft,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::SHIFT,
            },
            command_key(Key::B),
            egui::Event::Key {
                key: Key::ArrowLeft,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            command_key(Key::I),
            egui::Event::Text("尾".into()),
        ],
    ];
    for mode in [ViewMode::Edit, ViewMode::Hybrid, ViewMode::Split] {
        for (case, events) in scenarios.iter().enumerate() {
            let mut results = Vec::new();
            for batched in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let mut app = isolated_app(directory.path());
                app.new_document();
                app.state.view_mode = mode;
                app.session[0].content = "AB".into();
                app.session[0].update_after_edit();
                app.queue_editor_selection(0..2);
                let ctx = Context::default();
                install_fonts(&ctx);
                ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
                shortcut_editor_frame(&mut app, &ctx, vec![]);
                if batched {
                    shortcut_editor_frame(&mut app, &ctx, events.clone());
                } else {
                    for event in events.clone() {
                        shortcut_editor_frame(&mut app, &ctx, vec![event]);
                        drain_ordered_input(&mut app, &ctx);
                    }
                }
                drain_ordered_input(&mut app, &ctx);
                assert_eq!(ctx.options(|options| options.max_passes.get()), 1);
                if case == 0 {
                    assert_eq!(
                        app.session[0].content, "**[AB](https://)**",
                        "native Ctrl+K must insert a link, not delete the paragraph"
                    );
                }
                results.push(shortcut_history(&mut app));
            }
            assert_eq!(
                results[1], results[0],
                "mode={mode:?} case={case} events={events:?}"
            );
        }
    }
}

#[test]
fn ordered_shortcuts_bound_passes_and_preserve_new_input_after_overflow() {
    for mode in [ViewMode::Edit, ViewMode::Hybrid, ViewMode::Split] {
        let mut results = Vec::new();
        for batched in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = isolated_app(directory.path());
            app.new_document();
            app.state.view_mode = mode;
            app.session[0].content = "AB".into();
            app.session[0].update_after_edit();
            app.queue_editor_selection(1..1);
            let ctx = Context::default();
            install_fonts(&ctx);
            shortcut_editor_frame(&mut app, &ctx, vec![]);
            let events = (0..19)
                .flat_map(|_| [command_key(Key::B), egui::Event::Text("中".into())])
                .collect::<Vec<_>>();
            if batched {
                let output = shortcut_editor_frame(&mut app, &ctx, events);
                assert_eq!(
                    output.platform_output.num_completed_passes,
                    MAX_ORDERED_INPUT_PASSES
                );
                assert!(app.ordered_input_pass.is_some());
            } else {
                for event in events {
                    shortcut_editor_frame(&mut app, &ctx, vec![event]);
                }
            }
            shortcut_editor_frame(&mut app, &ctx, vec![egui::Event::Text("尾".into())]);
            drain_ordered_input(&mut app, &ctx);
            assert_eq!(app.session[0].content.matches('中').count(), 19);
            assert_eq!(app.session[0].content.matches('尾').count(), 1);
            results.push(shortcut_history(&mut app));
        }
        assert_eq!(results[1], results[0], "mode={mode:?}");
    }
}

fn shortcut_raw_frame(
    app: &mut RuporaApp,
    ctx: &Context,
    mut raw: egui::RawInput,
    visible: bool,
) -> egui::FullOutput {
    raw.screen_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1000.0, 800.0),
    ));
    eframe::App::raw_input_hook(app, ctx, &mut raw);
    ctx.run_ui(raw, |ui| {
        let mut frame = Frame::_new_kittest();
        eframe::App::logic(app, ctx, &mut frame);
        if visible {
            eframe::App::ui(app, ui, &mut frame);
        }
    })
}

#[test]
fn ordered_shortcuts_wait_for_visible_editor_before_acknowledging_input() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.state.view_mode = ViewMode::Edit;
    app.session[0].content = "AB".into();
    app.session[0].update_after_edit();
    app.queue_editor_selection(1..1);
    let ctx = Context::default();
    install_fonts(&ctx);
    shortcut_editor_frame(&mut app, &ctx, vec![]);
    shortcut_raw_frame(
        &mut app,
        &ctx,
        egui::RawInput {
            events: vec![
                egui::Event::Text("中".into()),
                command_key(Key::B),
                egui::Event::Text("文".into()),
            ],
            ..Default::default()
        },
        false,
    );
    assert_eq!(app.session[0].content, "AB");
    assert!(app.ordered_input_pass.as_ref().unwrap().stage.is_some());
    // A loss of native focus is later than the already received editor input.
    shortcut_raw_frame(
        &mut app,
        &ctx,
        egui::RawInput {
            focused: false,
            events: vec![egui::Event::WindowFocused(false)],
            ..Default::default()
        },
        false,
    );
    assert_eq!(app.session[0].content, "AB");
    for _ in 0..100 {
        shortcut_raw_frame(
            &mut app,
            &ctx,
            egui::RawInput {
                focused: false,
                ..Default::default()
            },
            false,
        );
    }
    assert_eq!(
        app.ordered_raw_input.len(),
        1,
        "empty unfocused frames must not grow the queue"
    );
    for x in 0..100 {
        shortcut_raw_frame(
            &mut app,
            &ctx,
            egui::RawInput {
                focused: false,
                events: vec![egui::Event::PointerMoved(egui::pos2(x as f32, 700.0))],
                ..Default::default()
            },
            false,
        );
    }
    assert_eq!(
        app.ordered_raw_input.len(),
        2,
        "hover packets coalesce without dropping the focus transition"
    );
    for event in [
        egui::Event::PointerButton {
            pos: egui::pos2(900.0, 700.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerMoved(egui::pos2(901.0, 701.0)),
        egui::Event::PointerMoved(egui::pos2(902.0, 702.0)),
        egui::Event::PointerButton {
            pos: egui::pos2(902.0, 702.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ] {
        shortcut_raw_frame(
            &mut app,
            &ctx,
            egui::RawInput {
                focused: false,
                events: vec![event],
                ..Default::default()
            },
            false,
        );
    }
    assert_eq!(
        app.ordered_raw_input.len(),
        6,
        "drag motion must retain its native packets"
    );
    shortcut_editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session[0].content, "A中**文**B");
    drain_ordered_input(&mut app, &ctx);
    assert_eq!(app.session[0].content, "A中**文**B");
}

#[test]
fn ordered_shortcuts_do_not_drain_root_input_from_child_viewports() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.queue_editor_selection(0..0);
    let ctx = Context::default();
    shortcut_editor_frame(&mut app, &ctx, vec![]);
    shortcut_editor_frame(&mut app, &ctx, vec![command_key(Key::Z); 12]);
    let remaining = app.ordered_input_pass.as_ref().unwrap().remaining.len();
    let mut child = egui::RawInput {
        viewport_id: egui::ViewportId::from_hash_of("child"),
        events: vec![egui::Event::Text("child".into())],
        ..Default::default()
    };
    eframe::App::raw_input_hook(&mut app, &ctx, &mut child);
    assert_eq!(child.events, vec![egui::Event::Text("child".into())]);
    assert_eq!(
        app.ordered_input_pass.as_ref().unwrap().remaining.len(),
        remaining
    );
    assert!(app.ordered_raw_input.is_empty());
}

#[test]
fn ordered_shortcuts_replay_close_once_after_pending_input_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.queue_editor_selection(0..0);
    let ctx = Context::default();
    shortcut_editor_frame(&mut app, &ctx, vec![]);
    shortcut_editor_frame(&mut app, &ctx, vec![command_key(Key::Z); 12]);
    assert!(app.ordered_input_pass.is_some());
    let mut close = egui::RawInput::default();
    close
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let output = shortcut_raw_frame(&mut app, &ctx, close, true);
    assert!(
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|command| matches!(command, ViewportCommand::CancelClose))
    );
    assert!(!app.allow_close);
    for expected in [
        ViewportCommand::Minimized(false),
        ViewportCommand::Visible(true),
        ViewportCommand::Focus,
    ] {
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&expected)
        );
    }
    let mut raw = egui::RawInput::default();
    eframe::App::raw_input_hook(&mut app, &ctx, &mut raw);
    let output = ctx.run_ui(raw, |ui| {
        let mut frame = Frame::_new_kittest();
        eframe::App::logic(&mut app, &ctx, &mut frame);
        eframe::App::ui(&mut app, ui, &mut frame);
        if ctx.current_pass_index() == 0 {
            ctx.request_discard("exercise close acknowledgement across passes");
        }
    });
    assert!(app.allow_close);
    assert_eq!(
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .filter(|command| matches!(command, ViewportCommand::Close))
            .count(),
        1
    );
    let output = shortcut_editor_frame(&mut app, &ctx, vec![]);
    assert!(
        !output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|command| matches!(command, ViewportCommand::Close))
    );
    assert!(app.ordered_raw_input.is_empty());
}

#[test]
fn ordered_shortcuts_shell_barriers_preserve_document_ownership() {
    let events = vec![
        egui::Event::Text("中".into()),
        command_key(Key::B),
        command_key(Key::N),
        egui::Event::Text("新".into()),
        command_key(Key::I),
        egui::Event::Text("尾".into()),
    ];
    let mut results = Vec::new();
    for batched in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.state.view_mode = ViewMode::Edit;
        app.session[0].content = "AB".into();
        app.session[0].update_after_edit();
        app.queue_editor_selection(1..1);
        let ctx = Context::default();
        install_fonts(&ctx);
        shortcut_editor_frame(&mut app, &ctx, vec![]);
        if batched {
            shortcut_editor_frame(&mut app, &ctx, events.clone());
            drain_ordered_input(&mut app, &ctx);
        } else {
            for event in events.clone() {
                shortcut_editor_frame(&mut app, &ctx, vec![event]);
                drain_ordered_input(&mut app, &ctx);
            }
        }
        results.push(
            app.session
                .documents()
                .iter()
                .map(|document| document.content.clone())
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(results[1], results[0]);
    assert_eq!(results[1][0], "A中****B");
}

#[test]
fn ordered_shortcuts_undo_precedes_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ordered-save.md");
    fs::write(&path, "AB").unwrap();
    let mut app = isolated_app(directory.path());
    app.session.insert(Document::open(&path).unwrap());
    app.restore_active_view_state();
    app.state.view_mode = ViewMode::Edit;
    app.queue_editor_selection(1..1);
    let ctx = Context::default();
    shortcut_editor_frame(&mut app, &ctx, vec![]);
    shortcut_editor_frame(&mut app, &ctx, vec![egui::Event::Text("中".into())]);
    shortcut_editor_frame(
        &mut app,
        &ctx,
        vec![command_key(Key::Z), command_key(Key::S)],
    );
    drain_ordered_input(&mut app, &ctx);
    assert_eq!(app.session[0].content, "AB");
    assert_eq!(fs::read_to_string(&path).unwrap(), "AB");
    assert!(!app.session[0].dirty);
}

#[test]
fn ordered_shortcuts_preserve_composition_and_escape_or_find_barriers() {
    for mode in [ViewMode::Edit, ViewMode::Hybrid] {
        for (case, events) in [
            vec![
                egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: "ni".into(),
                    active_range_chars: Some(0..2),
                }),
                command_key(Key::B),
                egui::Event::Ime(egui::ImeEvent::Commit("你".into())),
            ],
            vec![
                egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: "ni".into(),
                    active_range_chars: Some(0..2),
                }),
                command_key(Key::B),
                egui::Event::Ime(egui::ImeEvent::Commit("".into())),
            ],
            vec![
                egui::Event::Key {
                    key: Key::F3,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                command_key(Key::B),
                egui::Event::Text("你".into()),
            ],
            vec![
                egui::Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                command_key(Key::B),
                egui::Event::Text("你".into()),
            ],
        ]
        .into_iter()
        .enumerate()
        {
            let mut results = Vec::new();
            for batched in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let mut app = isolated_app(directory.path());
                app.new_document();
                app.state.view_mode = mode;
                app.find_query = "B".into();
                app.session[0].content = "AB".into();
                app.session[0].update_after_edit();
                app.queue_editor_selection(1..1);
                let ctx = Context::default();
                install_fonts(&ctx);
                shortcut_editor_frame(&mut app, &ctx, vec![]);
                if batched {
                    shortcut_editor_frame(&mut app, &ctx, events.clone());
                } else {
                    for event in events.clone() {
                        shortcut_editor_frame(&mut app, &ctx, vec![event]);
                        drain_ordered_input(&mut app, &ctx);
                    }
                }
                drain_ordered_input(&mut app, &ctx);
                let history = shortcut_history(&mut app);
                if mode == ViewMode::Hybrid && case < 2 {
                    assert!(
                        history.iter().all(|(source, _, _)| !source.contains("ni")),
                        "preedit is not document history: {history:?}"
                    );
                    assert_eq!(history[0].0.matches('你').count(), usize::from(case == 0));
                    assert!(history.iter().any(|(source, _, _)| source == "AB"));
                }
                results.push(history);
            }
            assert_eq!(results[1], results[0], "mode={mode:?} case={case}");
        }
    }
}

#[test]
fn ordered_shortcuts_respect_configured_f3_before_find_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.state.view_mode = ViewMode::Edit;
    app.state.key_bindings.bold = "F3".into();
    app.find_query = "B".into();
    app.session[0].content = "AB".into();
    app.session[0].update_after_edit();
    app.queue_editor_selection(1..1);
    let ctx = Context::default();
    shortcut_editor_frame(&mut app, &ctx, vec![]);
    shortcut_editor_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Text("中".into()),
            egui::Event::Key {
                key: Key::F3,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Text("文".into()),
        ],
    );
    drain_ordered_input(&mut app, &ctx);
    assert_eq!(app.session[0].content, "A中**文**B");
}

#[test]
fn middle_format_shortcut_keeps_input_order_in_the_same_context_run() {
    let mut failures = Vec::new();
    for mode in [ViewMode::Edit, ViewMode::Hybrid, ViewMode::Split] {
        for (key, expected, caret) in [
            (Key::B, "A中**文**B", 5),
            (Key::I, "A中*文*B", 4),
            (Key::K, "A中[文](https://)B", 4),
        ] {
            for prefix in [
                egui::Event::Text("中".into()),
                egui::Event::Paste("中".into()),
                egui::Event::Ime(egui::ImeEvent::Commit("中".into())),
            ] {
                for batched in [false, true] {
                    let directory = tempfile::tempdir().unwrap();
                    let mut app = isolated_app(directory.path());
                    app.new_document();
                    app.state.view_mode = mode;
                    app.session[0].content = "AB".into();
                    app.session[0].update_after_edit();
                    app.queue_editor_selection(1..1);
                    let ctx = Context::default();
                    install_fonts(&ctx);
                    shortcut_editor_frame(&mut app, &ctx, vec![]);
                    let events = vec![
                        prefix.clone(),
                        command_key(key),
                        egui::Event::Text("文".into()),
                    ];
                    if batched {
                        let output = shortcut_editor_frame(&mut app, &ctx, events);
                        // IME confirmation has its own transaction boundary
                        // before the format command; all stages still finish
                        // within this native frame.
                        let expected_passes = if matches!(prefix, egui::Event::Ime(_)) {
                            3
                        } else {
                            2
                        };
                        assert_eq!(output.platform_output.num_completed_passes, expected_passes);
                        assert!(app.ordered_input_pass.is_none());
                    } else {
                        for event in events {
                            shortcut_editor_frame(&mut app, &ctx, vec![event]);
                        }
                    }
                    let actual = (&app.session[0].content, app.active_selection(0));
                    if actual != (&expected.to_owned(), caret..caret) {
                        failures.push(format!("mode={mode:?} key={key:?} prefix={prefix:?} batched={batched} actual={actual:?} expected={expected:?}/{caret}"));
                    }
                    let edited = app.session[0].content.clone();
                    let selection = app.active_selection(0);
                    let mut undos = 0;
                    while app.session[0].can_undo() {
                        app.undo_active();
                        undos += 1;
                        assert!(undos <= 6);
                    }
                    assert_eq!(app.session[0].content, "AB");
                    for _ in 0..undos {
                        app.redo_active();
                    }
                    assert_eq!(app.session[0].content, edited);
                    assert_eq!(app.active_selection(0), selection);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn middle_format_shortcuts_reserve_the_required_passes_even_with_budget_one() {
    for mode in [ViewMode::Edit, ViewMode::Hybrid, ViewMode::Split] {
        for multiple_commands in [false, true] {
            let mut outcomes = Vec::new();
            for max_passes in [1, 2] {
                let directory = tempfile::tempdir().unwrap();
                let mut app = isolated_app(directory.path());
                app.new_document();
                app.state.view_mode = mode;
                app.session[0].content = "AB".into();
                app.session[0].update_after_edit();
                app.queue_editor_selection(1..1);
                let ctx = Context::default();
                ctx.options_mut(|options| options.max_passes = max_passes.try_into().unwrap());
                install_fonts(&ctx);
                shortcut_editor_frame(&mut app, &ctx, vec![]);
                let mut events = vec![
                    egui::Event::Text("中".into()),
                    command_key(Key::B),
                    egui::Event::Text("文".into()),
                ];
                if multiple_commands {
                    events.extend([command_key(Key::I), egui::Event::Text("尾".into())]);
                }
                shortcut_editor_frame(&mut app, &ctx, events);
                assert!(app.ordered_input_pass.is_none());
                for character in if multiple_commands {
                    "中文尾"
                } else {
                    "中文"
                }
                .chars()
                {
                    assert_eq!(app.session[0].content.matches(character).count(), 1);
                }
                let source = app.session[0].content.clone();
                shortcut_editor_frame(&mut app, &ctx, vec![]);
                assert_eq!(app.session[0].content, source, "no next-frame replay");
                outcomes.push(source);
            }
            if multiple_commands {
                assert_eq!(
                    outcomes[0], outcomes[1],
                    "pass budget cannot change command semantics"
                );
            } else {
                assert_eq!(outcomes[0], "A中**文**B");
                assert_eq!(outcomes[1], "A中**文**B");
            }
        }
    }
}

#[test]
fn middle_format_shortcut_preserves_commit_and_following_ime_composition() {
    for mode in [ViewMode::Edit, ViewMode::Hybrid, ViewMode::Split] {
        for cancel in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = isolated_app(directory.path());
            app.new_document();
            app.state.view_mode = mode;
            app.session[0].content = "AB".into();
            app.session[0].update_after_edit();
            app.queue_editor_selection(1..1);
            let ctx = Context::default();
            install_fonts(&ctx);
            shortcut_editor_frame(&mut app, &ctx, vec![]);
            shortcut_editor_frame(
                &mut app,
                &ctx,
                vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: "ni".into(),
                    active_range_chars: Some(0..2),
                })],
            );
            shortcut_editor_frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::Ime(egui::ImeEvent::Commit("中".into())),
                    command_key(Key::B),
                    egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: "wen".into(),
                        active_range_chars: Some(0..3),
                    }),
                ],
            );
            assert!(app.ordered_input_pass.is_none());
            shortcut_editor_frame(
                &mut app,
                &ctx,
                vec![egui::Event::Ime(egui::ImeEvent::Commit(
                    if cancel { "" } else { "文" }.into(),
                ))],
            );
            assert_eq!(
                app.session[0].content,
                if cancel { "A中****B" } else { "A中**文**B" }
            );
        }
    }
}

#[test]
fn middle_format_shortcut_finishes_on_its_original_document() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = isolated_app(directory.path());
    app.new_document();
    app.session[0].content = "AB".into();
    app.session[0].update_after_edit();
    app.new_document();
    app.session[1].content = "KEEP".into();
    app.session[1].update_after_edit();
    app.activate_document(0);
    app.queue_editor_selection(1..1);
    let ctx = Context::default();
    install_fonts(&ctx);
    shortcut_editor_frame(&mut app, &ctx, vec![]);
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            events: vec![
                egui::Event::Text("中".into()),
                command_key(Key::B),
                egui::Event::Text("文".into()),
            ],
            ..Default::default()
        },
        |ui| {
            let mut frame = Frame::_new_kittest();
            eframe::App::logic(&mut app, &ctx, &mut frame);
            eframe::App::ui(&mut app, ui, &mut frame);
            if ctx.current_pass_index() == 0 {
                assert!(app.ordered_input_pass.is_some());
                // Even an unexpected internal view change cannot redirect text.
                app.activate_document(1);
                app.close_document(0);
                app.last_recovery_write = Instant::now() - Duration::from_secs(60);
            }
        },
    );
    assert!(app.ordered_input_pass.is_none());
    assert_eq!(app.session[0].content, "A中**文**B");
    assert_eq!(app.session[1].content, "KEEP");
    assert_eq!(app.session.active_index(), Some(0));
    assert_eq!(app.active_selection(0), 5..5);
    assert!(
        app.last_recovery_write.elapsed() >= Duration::from_secs(60),
        "supplemental logic must not run recovery/background work"
    );
}

#[test]
fn trailing_format_shortcut_without_editor_focus_uses_immediate_route() {
    for mode in [ViewMode::Edit, ViewMode::Hybrid, ViewMode::Preview] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.state.view_mode = ViewMode::Edit;
        app.session[0].content = "AB".into();
        app.session[0].update_after_edit();
        app.queue_editor_selection(1..1);
        let ctx = Context::default();
        install_fonts(&ctx);
        shortcut_editor_frame(&mut app, &ctx, vec![]);
        app.state.view_mode = mode;
        ctx.memory_mut(|memory| memory.surrender_focus(app.editor_surface.widget_id().unwrap()));
        shortcut_editor_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("中".into()), command_key(Key::B)],
        );
        assert_ne!(app.state.view_mode, ViewMode::Preview);
        assert_eq!(app.session[0].content, "A**中**B", "mode={mode:?}");
        shortcut_editor_frame(&mut app, &ctx, vec![]);
        assert_eq!(
            app.session[0].content, "A**中**B",
            "no deferred command remains"
        );
    }
}

#[test]
fn middle_format_shortcut_keeps_window_and_pointer_events_on_the_original_route() {
    for scenario in [
        "popup",
        "find",
        "drag",
        "touch",
        "window-focus",
        "drop",
        "close",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.session[0].content = "AB".into();
        app.session[0].update_after_edit();
        app.queue_editor_selection(1..1);
        let ctx = Context::default();
        install_fonts(&ctx);
        shortcut_editor_frame(&mut app, &ctx, vec![]);
        let mut input = egui::RawInput {
            events: vec![
                egui::Event::Text("中".into()),
                command_key(Key::B),
                egui::Event::Text("文".into()),
            ],
            ..Default::default()
        };
        match scenario {
            "popup" => egui::Popup::open_id(&ctx, egui::Id::new("existing-menu")),
            "find" => app.find_open = true,
            "pointer" => input
                .events
                .push(egui::Event::PointerMoved(egui::pos2(900.0, 700.0))),
            "drag" => input.events.push(egui::Event::PointerButton {
                pos: egui::pos2(900.0, 700.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }),
            "touch" => input.events.push(egui::Event::Touch {
                device_id: egui::TouchDeviceId(1),
                id: egui::TouchId(1),
                phase: egui::TouchPhase::Start,
                pos: egui::pos2(900.0, 700.0),
                force: None,
            }),
            "window-focus" => input.events.push(egui::Event::WindowFocused(false)),
            "drop" => input.dropped_files.push(egui::DroppedFile {
                path: Some(directory.path().join("drop.md")),
                ..Default::default()
            }),
            "close" => input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .events
                .push(egui::ViewportEvent::Close),
            _ => unreachable!(),
        }
        let output = ctx.run_ui(input, |_| {
            app.handle_shortcuts(&ctx);
            assert!(app.ordered_input_pass.is_none(), "scenario={scenario}");
            let text_events = ctx.input(|input| {
                input
                    .events
                    .iter()
                    .filter(|event| matches!(event, egui::Event::Text(_)))
                    .count()
            });
            assert_eq!(
                text_events, 2,
                "both text events must remain available: {scenario}"
            );
        });
        assert!(
            !output
                .platform_output
                .request_discard_reasons
                .iter()
                .any(|cause| cause.reason.contains("format shortcut"))
        );
    }
}

#[test]
fn trailing_format_does_not_replace_another_app_shortcut() {
    for new_document_position in [0, 1] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.session[0].content = "AB".into();
        app.session[0].update_after_edit();
        let mut events = vec![egui::Event::Text("中".into())];
        events.insert(
            new_document_position,
            egui::Event::Key {
                key: Key::N,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
        );
        events.push(egui::Event::Key {
            key: Key::B,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        let ctx = Context::default();
        let _ = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |_| {
                app.handle_shortcuts(&ctx);
            },
        );
        assert_eq!(
            app.session.documents().len(),
            2,
            "earlier New command must retain its existing priority"
        );
        assert_eq!(app.session[0].content, "AB");
    }
}

#[test]
fn trailing_format_shortcuts_preserve_preceding_input_order() {
    fn draw(app: &mut RuporaApp, ctx: &Context, hybrid: bool, events: Vec<egui::Event>) {
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                app.handle_shortcuts(ctx);
                if hybrid {
                    app.hybrid_pane(ui, 0);
                } else {
                    app.edit_pane(ui, 0, None);
                }
            },
        );
    }
    let mut failures = Vec::new();
    for hybrid in [false, true] {
        for (key, wrapper) in [(Key::B, "****"), (Key::I, "**"), (Key::K, "[](https://)")] {
            for custom_binding in [false, true] {
                for prefix in [
                    egui::Event::Text("中".into()),
                    egui::Event::Paste("中".into()),
                    egui::Event::Ime(egui::ImeEvent::Commit("中".into())),
                    egui::Event::Key {
                        key: Key::ArrowRight,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ] {
                    let expected = if matches!(prefix, egui::Event::Key { .. }) {
                        format!("AB{wrapper}")
                    } else {
                        format!("A中{wrapper}B")
                    };
                    let mut outcomes = Vec::new();
                    for batched in [false, true] {
                        let directory = tempfile::tempdir().unwrap();
                        let mut app = isolated_app(directory.path());
                        app.new_document();
                        app.session[0].content = "AB".into();
                        app.session[0].update_after_edit();
                        app.queue_editor_selection(1..1);
                        if custom_binding {
                            app.state.key_bindings.bold = "Ctrl+Shift+B".into();
                            app.state.key_bindings.italic = "Ctrl+Shift+I".into();
                            app.state.key_bindings.link = "Ctrl+Shift+K".into();
                        }
                        let ctx = Context::default();
                        install_fonts(&ctx);
                        draw(&mut app, &ctx, hybrid, vec![]);
                        let events = vec![
                            prefix.clone(),
                            egui::Event::Key {
                                key,
                                physical_key: Some(key),
                                pressed: true,
                                repeat: false,
                                modifiers: egui::Modifiers {
                                    ctrl: true,
                                    command: true,
                                    shift: custom_binding,
                                    ..Default::default()
                                },
                            },
                        ];
                        if batched {
                            draw(&mut app, &ctx, hybrid, events);
                        } else {
                            for event in events {
                                draw(&mut app, &ctx, hybrid, vec![event]);
                            }
                        }
                        if app.session[0].content != expected {
                            failures.push(format!("hybrid={hybrid} key={key:?} custom_binding={custom_binding} prefix={prefix:?} batched={batched} expected={expected:?} actual={:?}", app.session[0].content));
                        }
                        let edited = app.session[0].content.clone();
                        let selection = app.active_selection(0);
                        let mut undo_count = 0;
                        while app.session[0].can_undo() {
                            app.undo_active();
                            undo_count += 1;
                            assert!(undo_count < 8);
                        }
                        assert_eq!(app.session[0].content, "AB");
                        for _ in 0..undo_count {
                            app.redo_active();
                        }
                        assert_eq!(app.session[0].content, edited);
                        assert_eq!(app.active_selection(0), selection);
                        outcomes.push((edited, selection));
                    }
                    if outcomes[0] != outcomes[1] {
                        failures.push(format!("hybrid={hybrid} key={key:?} prefix={prefix:?} separate={:?} batched={:?}", outcomes[0], outcomes[1]));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

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

#[test]
fn selected_save_path_does_not_overwrite_a_concurrent_creator() {
    for encoding in [None, Some(TextEncoding::Utf8)] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        let original = directory.path().join("utf16-original.md");
        let original_bytes = [0xff, 0xfe]
            .into_iter()
            .chain(
                "original\r\n内容\r\n"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes),
            )
            .collect::<Vec<_>>();
        fs::write(&original, &original_bytes).unwrap();
        app.open_paths([original.clone()]);
        assert_eq!(app.session[0].encoding, TextEncoding::Utf16Le);
        assert_eq!(
            app.session[0].line_ending,
            crate::document::LineEnding::CrLf
        );
        app.session[0].edit(EditKind::Typing, None, |text| {
            *text = "local draft\n修改\n".to_owned();
            None
        });
        let snapshot = app.session[0].snapshot();
        let path = directory.path().join("new.md");
        assert!(!path.exists());
        crate::document::CREATE_TARGET_BEFORE_ATOMIC_PERSIST.with(|create| create.set(true));

        let result = app.save_to_selected_path(0, path.clone(), encoding);

        assert_eq!(fs::read_to_string(path).unwrap(), "concurrent creator");
        assert!(result.is_err());
        assert_eq!(app.session[0].snapshot(), snapshot);
        // Opening resolves short paths and symlinks before saving starts.
        assert_eq!(app.session[0].path.as_deref(), snapshot.path());
        assert_eq!(app.session[0].encoding, TextEncoding::Utf16Le);
        assert_eq!(
            app.session[0].line_ending,
            crate::document::LineEnding::CrLf
        );
        assert_eq!(fs::read(&original).unwrap(), original_bytes);
        assert!(app.session[0].dirty);
        assert!(app.session[0].can_undo());
        assert!(Document::open(&original).is_err());
    }
}

#[test]
fn selected_save_path_preserves_confirmed_existing_target_semantics() {
    for encoding in [None, Some(TextEncoding::Utf8)] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.session[0].edit(EditKind::Typing, None, |text| {
            *text = "confirmed replacement".to_owned();
            None
        });
        let path = directory.path().join("existing.md");
        fs::write(&path, "old target").unwrap();

        app.save_to_selected_path(0, path.clone(), encoding)
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "confirmed replacement");
        assert!(!app.session[0].dirty);
        assert!(Document::open(&path).is_err());
    }
}

#[test]
fn editor_ime_candidate_area_tracks_the_caret_in_a_tall_editor() {
    for mode in [ViewMode::Edit, ViewMode::Split, ViewMode::Hybrid] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = isolated_app(directory.path());
        app.new_document();
        app.session[0].content = "A🙂B".into();
        app.session[0].update_after_edit();
        app.queue_editor_selection(1..1);
        app.state.view_mode = mode;
        let ctx = Context::default();
        install_fonts(&ctx);
        shortcut_editor_frame(&mut app, &ctx, vec![]);
        let output = shortcut_editor_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "zhong'wen".into(),
                active_range_chars: Some(0..8),
            })],
        );
        let ime = output
            .platform_output
            .ime
            .expect("focused editor enables IME");
        assert!(
            (ime.rect.bottom() - ime.cursor_rect.bottom()).abs() <= 1.0,
            "mode={mode:?}: candidate area {:?} must end at caret {:?}, not at the bottom of the editor",
            ime.rect,
            ime.cursor_rect,
        );
    }
}
