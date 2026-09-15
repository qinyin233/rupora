//! Projected block editing, IME, navigation and cross-block selection.
use super::*;

impl EditorSurface {
    fn finish_leading_ime_cancel(&mut self, ui: &mut Ui, document: &Document) {
        let Some(editor_id) = self.editor_widget_id else {
            return;
        };
        if !self
            .hybrid_ime_session
            .as_ref()
            .is_some_and(|session| session.document_id == document.id())
            || ui.memory(|memory| memory.focused()) != Some(editor_id)
        {
            return;
        }
        let cancel = ui.input(|input| {
            for (index, event) in input.events.iter().enumerate() {
                match event {
                    egui::Event::Ime(
                        egui::ImeEvent::Preedit { text, .. } | egui::ImeEvent::Commit(text),
                    ) if text.is_empty() => return Some(index),
                    egui::Event::Ime(egui::ImeEvent::Preedit { .. })
                    | egui::Event::Key {
                        key: Key::Escape, ..
                    } => {}
                    egui::Event::Text(_)
                    | egui::Event::Paste(_)
                    | egui::Event::Cut
                    | egui::Event::Copy
                    | egui::Event::Ime(egui::ImeEvent::Commit(_))
                    | egui::Event::Key { pressed: true, .. }
                    | egui::Event::PointerButton { pressed: true, .. } => return None,
                    _ => {}
                }
            }
            None
        });
        let Some(cancel) = cancel else {
            return;
        };
        // Restore the document buffer before processing subsequent input. IME
        // replacement text (including a replaced selection) is only provisional.
        self.hybrid_ime_session = None;
        egui::text_edit::TextEditState::default().store(ui.ctx(), editor_id);
        if let Some(cursor) = self.editor_cursor {
            let start = cursor_range_to_char_range(cursor).start;
            self.queue_editor_selection(start..start);
        }
        ui.input_mut(|input| {
            input.events.drain(..=cancel);
        });
    }

    fn apply_hybrid_cross_selection_input(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        effects: &mut EditorOutput,
    ) {
        let document_id = document.id();
        let Some(selection) = self
            .hybrid_cross_selection
            .filter(|selection| selection.document_id == document_id)
        else {
            self.hybrid_cross_selection = None;
            return;
        };
        let range = cursor_range_to_char_range(selection.cursor);
        if range.is_empty() {
            self.hybrid_cross_selection = None;
            return;
        }
        let before = document.content.clone();
        let range = clamp_char_range(&before, range);
        let start = char_to_byte(&before, range.start);
        let end = char_to_byte(&before, range.end);
        let action = loop {
            let Some(action) = take_cross_block_input(ui) else {
                return;
            };
            if action.copy() {
                ui.ctx().copy_text(before[start..end].to_owned());
            }
            if action.replacement().is_some() {
                break action;
            }
            // Copy leaves the selection active. A subsequent edit in this frame
            // must still operate on the full range, before a block-local widget.
        };
        let mut replacement = action.replacement().expect("replacement action").to_owned();
        let cursor = range.start + replacement.chars().count();
        if !replacement.ends_with('\n')
            && !before[..start].ends_with('\n')
            && document.blocks().iter().any(|block| {
                block.range.start == end && is_fenced_code_block(&before[block.range.clone()])
            })
        {
            replacement.push('\n');
        }

        document.content.replace_range(start..end, &replacement);
        let cursor = CCursorRange::one(CCursor::new(cursor));
        document.record_edit(
            before,
            Some(range),
            Some(cursor_range_to_char_range(cursor)),
            EditKind::Typing,
        );
        self.editor_cursor = Some(cursor);
        self.pending_editor_cursor = Some(cursor);
        self.hybrid_cross_selection = None;
        self.hybrid_pointer_anchor = None;
        self.hybrid_ime_session = None;
        effects.notice = "已更新跨块选择".to_owned();
    }

    /// Apply structural input before constructing block widgets, so each Enter
    /// reparses its new paragraphs before the next event in the same frame.
    fn apply_leading_hybrid_enter(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        event_index: usize,
    ) -> bool {
        if self.hybrid_ime_session.is_some() || self.hybrid_cross_selection.is_some() {
            return false;
        }
        let event = ui.input(|input| input.events[event_index].clone());
        let (text, enter, shift) = match event {
            egui::Event::Key {
                key: Key::Enter,
                modifiers,
                ..
            } if !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd => {
                ("\n".to_owned(), true, modifiers.shift)
            }
            egui::Event::Text(text) | egui::Event::Paste(text)
                if ui.input(|input| {
                    input.events[event_index + 1..].iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::Key {
                                key: Key::Enter,
                                pressed: true,
                                ..
                            }
                        )
                    })
                }) =>
            {
                (text, false, false)
            }
            _ => return false,
        };
        let Some(cursor) = self.pending_editor_cursor.or(self.editor_cursor) else {
            return false;
        };
        let before = document.content.clone();
        let selected = cursor_range_to_char_range(cursor);
        let blocks = document.blocks().to_vec();
        let block = block_for_char_index(&before, &blocks, selected.start);
        if char_to_byte(&before, selected.end) > block.range.end {
            return false;
        }
        let block_index = blocks.iter().position(|item| item.id == block.id).unwrap();
        let original = &before[block.range.clone()];
        let block_start = before[..block.range.start].chars().count();
        let local =
            selected.start.saturating_sub(block_start)..selected.end.saturating_sub(block_start);
        let projection = VisualProjection::from_markdown_with_context(
            original,
            Some(local.clone()),
            document.references(),
        );
        let visual_selection = projection.visual_char_range(original, local);
        let mut visual = projection.text().to_owned();
        let paired = (!enter)
            .then(|| editing::apply_smart_pair(&mut visual, visual_selection.clone(), &text))
            .flatten();
        let selection = paired.unwrap_or_else(|| {
            visual.replace_range(
                char_to_byte(&visual, visual_selection.start)
                    ..char_to_byte(&visual, visual_selection.end),
                &text,
            );
            let at = visual_selection.start + text.chars().count();
            at..at
        });
        let Some(mut update) = projection.apply_edit(original, &visual, selection) else {
            return false;
        };
        if enter {
            if !shift
                && let Some(selection) =
                    complete_fenced_code_on_enter(&mut update.source, update.selection.clone())
            {
                update.selection = selection;
            } else if !is_fenced_code_block(original) {
                update.selection =
                    complete_visual_enter(&mut update.source, update.selection, shift);
            }
        } else if let Some(selection) = consume_paired_fenced_code_closer(
            &mut update.source,
            update.selection.clone(),
        )
        .or_else(|| {
            complete_bare_fenced_code_after_typing(&mut update.source, update.selection.clone())
        }) {
            update.selection = selection;
        }
        if block.range.is_empty() && !update.source.trim().is_empty() {
            let (prefix, suffix) = empty_paragraph_separators(&before, &blocks, block_index);
            update.source = format!(
                "{}{}{}",
                "\n".repeat(prefix),
                update.source,
                "\n".repeat(suffix)
            );
            update.selection = update.selection.start + prefix..update.selection.end + prefix;
        }
        let next = block_start + update.selection.start..block_start + update.selection.end;
        document
            .content
            .replace_range(block.range.clone(), &update.source);
        document.record_edit(before, Some(selected), Some(next.clone()), EditKind::Typing);
        self.queue_editor_selection(next);
        ui.input_mut(|input| {
            input.events.remove(event_index);
        });
        ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        true
    }

    fn prepare_hybrid_navigation(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        options: &EditorOptions<'_>,
    ) {
        loop {
            if self.apply_leading_tab_input(ui, document) {
                continue;
            }
            let Some(event_index) = leading_editor_event(ui, self.editor_widget_id) else {
                break;
            };
            if self.apply_leading_hybrid_enter(ui, document, event_index) {
                continue;
            }
            let Some(egui::Event::Key { key, modifiers, .. }) =
                ui.input(|input| input.events.get(event_index).cloned())
            else {
                break;
            };
            let Some(cursor) = self.pending_editor_cursor.or(self.editor_cursor) else {
                break;
            };
            let blocks = document.blocks().to_vec();
            let references = document.references();
            let source = &document.content;
            let selected = cursor_range_to_char_range(cursor);
            let block = block_for_char_index(source, &blocks, cursor.primary.index.0);
            let block_index = blocks.iter().position(|item| item.id == block.id).unwrap();
            let cross = self.hybrid_cross_selection.is_some()
                || !selected.is_empty()
                    && block_for_char_index(source, &blocks, selected.start).id
                        != block_for_char_index(source, &blocks, selected.end - 1).id;
            let target = if !modifiers.command && matches!(key, Key::PageUp | Key::PageDown) {
                Some(hybrid_page_cursor(
                    ui,
                    source,
                    &blocks,
                    cursor.primary.index.0,
                    key == Key::PageDown,
                    options.dark,
                ))
            } else if modifiers.command && matches!(key, Key::Home | Key::End) {
                Some(if !modifiers.shift {
                    hybrid_document_edge(source, &blocks, key == Key::End)
                } else if key == Key::Home {
                    0
                } else {
                    source.chars().count()
                })
            } else if cross && !modifiers.shift && matches!(key, Key::ArrowLeft | Key::ArrowRight) {
                Some(if key == Key::ArrowLeft {
                    selected.start
                } else {
                    selected.end
                })
            } else if cross && matches!(key, Key::Home | Key::End) {
                let byte = char_to_byte(source, cursor.primary.index.0);
                let target = if key == Key::Home {
                    source[..byte].rfind('\n').map_or(0, |at| at + 1)
                } else {
                    source[byte..]
                        .find('\n')
                        .map_or(source.len(), |at| byte + at)
                };
                Some(source[..target].chars().count())
            } else if selected.is_empty() && !modifiers.shift {
                let start = source[..block.range.start].chars().count();
                fenced_boundary_delete_cursor(
                    source,
                    &blocks,
                    block_index,
                    selected.start.saturating_sub(start)..selected.end.saturating_sub(start),
                    key == Key::Backspace,
                    key == Key::Delete,
                )
                .or_else(|| {
                    if modifiers.ctrl || modifiers.alt || modifiers.mac_cmd {
                        return None;
                    }
                    hybrid_boundary_arrow_cursor(
                        source,
                        &blocks,
                        block_index,
                        selected.start,
                        key,
                        references.clone(),
                    )
                })
            } else {
                None
            };
            if target.is_none()
                && modifiers.is_none()
                && selected.is_empty()
                && let Some(range) = paragraph_boundary_delete_range(
                    source,
                    &blocks,
                    block_index,
                    selected.start,
                    key,
                )
            {
                let next = source[..range.start].chars().count();
                let before = source.clone();
                document.content.replace_range(range, "");
                document.record_edit(before, Some(selected), Some(next..next), EditKind::Typing);
                self.queue_editor_selection(next..next);
                ui.input_mut(|input| {
                    input.events.remove(event_index);
                });
                continue;
            }
            let Some(target) = target else {
                break;
            };
            ui.input_mut(|input| {
                input.events.remove(event_index);
            });
            // egui records spatial focus movement before this handler runs.
            // The arrow has moved the document caret; do not also focus a
            // neighboring preview when the new editor is first created.
            ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            let next = if modifiers.shift {
                CCursorRange {
                    primary: CCursor::new(target),
                    secondary: cursor.secondary,
                    h_pos: None,
                }
            } else {
                CCursorRange::one(CCursor::new(target))
            };
            self.queue_editor_selection(cursor_range_to_char_range(next));
            self.editor_cursor = Some(next);
            self.pending_editor_cursor = Some(next);
        }
    }

    pub(super) fn hybrid_pane(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        options: &EditorOptions<'_>,
        effects: &mut EditorOutput,
    ) {
        normalize_editor_input_line_endings(ui);
        self.finish_leading_ime_cancel(ui, document);
        self.prepare_hybrid_navigation(ui, document, options);
        if has_leading_select_all(ui, self.editor_widget_id) {
            let blocks = document.blocks().to_vec();
            let source = &document.content;
            let cursor = self
                .editor_cursor
                .map_or(0, |cursor| cursor.primary.index.0);
            let block = block_for_char_index(source, &blocks, cursor);
            let start = source[..block.range.start].chars().count();
            let end = source[..block.range.end].chars().count();
            let projection = VisualProjection::from_markdown(&source[block.range.clone()]);
            let selected = self
                .editor_cursor
                .map(cursor_range_to_char_range)
                .unwrap_or(cursor..cursor);
            let visual = projection.visual_char_range(
                &source[block.range.clone()],
                selected.start.saturating_sub(start)..selected.end.saturating_sub(start),
            );
            let select_document =
                !selected.is_empty() && visual == (0..projection.text().chars().count());
            if select_document || !is_fenced_code_block(&source[block.range.clone()]) {
                let selection = if select_document {
                    0..source.chars().count()
                } else {
                    start..end
                };
                take_leading_select_all(ui, self.editor_widget_id);
                self.queue_editor_selection(selection);
                effects.notice = if select_document {
                    "已全选文档"
                } else {
                    "已选择当前块；再次按 Ctrl+A 全选文档"
                }
                .to_owned();
            }
        }
        self.apply_leading_tab_input(ui, document);
        if let Some(cursor) = self.pending_editor_cursor {
            let blocks = document.blocks().to_vec();
            let source = &document.content;
            let range = clamp_char_range(source, cursor_range_to_char_range(cursor));
            let first_block = block_for_char_index(source, &blocks, range.start);
            self.hybrid_cross_selection = (!range.is_empty()
                && (first_block.id != block_for_char_index(source, &blocks, range.end - 1).id
                    || char_to_byte(source, range.end) > first_block.range.end))
                .then_some(HybridCrossSelection {
                    document_id: document.id(),
                    cursor,
                });
        }
        self.apply_hybrid_cross_selection_input(ui, document, effects);
        self.render_cache.begin_frame();
        let viewport_height = ui.available_height();
        let document_id = document.id();
        let document_title = document.title();
        let cursor_before = self.editor_cursor;
        let selection_before = cursor_before.map(cursor_range_to_char_range);
        let view = document.render_view();
        let source = view.source;
        let blocks = view.blocks;
        let references = view.references;
        let base_path = options.base_path;
        let prepared_document =
            self.render_cache
                .prepare_document(document_id, source, blocks, references.clone());
        let dark = options.dark;
        let screen_reader = ui.ctx().options(|options| options.screen_reader);

        let mut pending_source_cursor = self.pending_editor_cursor.take();
        if let Some(cursor_range) = pending_source_cursor {
            let [selection_start, _] = cursor_range.sorted_cursors();
            let selected_block = block_for_char_index(source, blocks, selection_start.index.0);
            self.hybrid_active = Some((document_id, selected_block.id));
        }

        let active_id = self
            .hybrid_active
            .filter(|(document, id)| {
                *document == document_id && blocks.iter().any(|block| block.id == *id)
            })
            .map(|(_, id)| id)
            .or_else(|| source.is_empty().then(|| blocks[0].id));
        let ime_action = ui.input(|input| ime_frame_action(&input.events));
        let ime_has_commit = ui.input(|input| {
            input.events.iter().any(|event| {
            matches!(event, egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty())
        })
        });
        let mut ime_session = self.hybrid_ime_session.take().filter(|session| {
            session.document_id == document_id && Some(session.block_id) == active_id
        });

        let mut pending_edit = None;
        let mut activate = None;
        let mut next_global_cursor = None;
        let mut cursor_adjusted = false;
        let mut page_rect = None;
        let mut active_editor_rect = None;
        let mut pointer_regions = Vec::<HybridPointerRegion>::new();
        let mut copied_code_block = false;
        let mut clicked_destination = None;
        let palette = app_palette(options.dark);

        ScrollArea::vertical()
            .id_salt(("hybrid-scroll", document_id))
            .show(ui, |ui| {
                let page_layout = document_page_layout(ui.available_width(), viewport_height);
                ui.add_space(page_layout.top_margin);
                ui.horizontal(|ui| {
                    ui.add_space(page_layout.side_margin);
                    let page = document_page_frame(palette, options.dark, &page_layout)
                        .show(ui, |ui| {
                            set_wysiwyg_document_accessibility(ui, &document_title);
                            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                ui.set_width(page_layout.content_width);
                                ui.set_min_height(page_layout.min_content_height);
                                for (block_index, block) in blocks.iter().enumerate() {
                                    let source_block = &source[block.range.clone()];
                                    let code_content = fenced_code_content(source_block);
                                    let block_is_code = code_content.is_some();
                                    let block_layout_start = ui.next_widget_position().y;
                                    let prepared_block = prepared_document.block(ui.ctx(), block, ui.available_width(), dark, base_path);
                                    let block_height = prepared_block.estimated_height();
                                    let predicted_rect = egui::Rect::from_min_size(
                                        egui::pos2(ui.min_rect().left(), block_layout_start),
                                        egui::vec2(ui.available_width(), block_height),
                                    );
                                    let prefetch_rect = ui
                                        .clip_rect()
                                        .expand2(egui::vec2(0.0, viewport_height.max(200.0)));
                                    if !screen_reader
                                        && Some(block.id) != active_id
                                        && !prefetch_rect.intersects(predicted_rect)
                                    {
                                        ui.add_space(block_height);
                                        continue;
                                    }

                                    let generated_preview = prepared_block.is_generated();
                                    let preview_range = block.range.clone();
                                    let preview_source = prepared_block.preview_source();
                                    let mut code_surface_rect = None;
                                    ui.push_id(("hybrid-block", document_id, block.id), |ui| {
                                        if Some(block.id) == active_id {
                                            let mut edit_range =
                                                hybrid_edit_range(source, blocks, block.id);
                                            if let Some(cursor) = cursor_before {
                                                let byte = char_to_byte(source, cursor.primary.index.0);
                                                if byte < edit_range.start
                                                    && block_index > 0
                                                    && fenced_code_content(&source[blocks[block_index - 1].range.clone()]).is_some()
                                                    && byte >= blocks[block_index - 1].range.end
                                                {
                                                    edit_range.start = byte;
                                                }
                                            }
                                            // Select All is local to the editable block. Exclude
                                            // inter-block separators before TextEdit processes the
                                            // event batch, so a following edit cannot erase them.
                                            if !block_is_code && editor_input_action(ui).select_all {
                                                edit_range.end = block.range.end;
                                            }
                                            let block_char_start =
                                                source[..edit_range.start].chars().count();
                                            let local_source_cursor_before =
                                                cursor_before.map(|cursor| {
                                                    cursor_range_saturating_sub(
                                                        cursor,
                                                        block_char_start,
                                                    )
                                                });
                                            let local_source_selection_before =
                                                local_source_cursor_before
                                                    .map(cursor_range_to_char_range);
                                            let (original_block, ime_visual_content) =
                                                if let Some(session) = ime_session.take() {
                                                    (
                                                        session.base_source,
                                                        Some(session.visual_content),
                                                    )
                                                } else {
                                                    (source[edit_range.clone()].to_owned(), None)
                                                };
                                            let had_ime_session = ime_visual_content.is_some();
                                            let projection = VisualProjection::from_markdown_with_context(
                                                &original_block, local_source_selection_before.clone(), references.clone(),
                                            );
                                            let mut visual_content = ime_visual_content
                                                .unwrap_or_else(|| projection.text().to_owned());
                                            let visual_selection_before =
                                                local_source_selection_before.as_ref().map(
                                                    |selection| {
                                                        projection.visual_char_range(
                                                            &original_block,
                                                            selection.clone(),
                                                        )
                                                    },
                                                );
                                            let editor_id = ui.make_persistent_id((
                                                "hybrid-editor",
                                                document_id,
                                                block.id,
                                            ));
                                            self.editor_widget_id = Some(editor_id);
                                            let pending_local_cursor =
                                                pending_source_cursor.take().map(|cursor_range| {
                                                    cursor_range_saturating_sub(
                                                        cursor_range,
                                                        block_char_start,
                                                    )
                                                });
                                            let cursor_source_cursor =
                                                if pending_local_cursor.is_some() {
                                                    pending_local_cursor.as_ref()
                                                } else if had_ime_session {
                                                    None
                                                } else {
                                                    local_source_cursor_before.as_ref()
                                                };
                                            if let Some(local_cursor) = cursor_source_cursor {
                                                let local_source =
                                                    cursor_range_to_char_range(*local_cursor);
                                                let local_visual = projection.visual_char_range(
                                                    &original_block,
                                                    local_source,
                                                );
                                                let mut state =
                                                    TextEdit::load_state(ui.ctx(), editor_id)
                                                        .unwrap_or_default();
                                                state.cursor.set_char_range(Some(
                                                    cursor_range_with_direction(
                                                        local_visual,
                                                        *local_cursor,
                                                    ),
                                                ));
                                                state.store(ui.ctx(), editor_id);
                                            }
                                            if pending_local_cursor.is_some() {
                                                ui.memory_mut(|memory| {
                                                    memory.request_focus(editor_id)
                                                });
                                            }

                                            let block_is_code =
                                                is_fenced_code_block(&original_block);
                                            let code_language =
                                                fenced_code_language(&original_block)
                                                    .map(str::to_owned);
                                            let block_is_table =
                                                is_native_table_block(&original_block);
                                            let block_is_quote =
                                                is_native_quote_block(&original_block);
                                            let frame = if block_is_code {
                                                code_block_frame(palette)
                                            } else if block_is_table {
                                                egui::Frame::new()
                                                    .fill(palette.code_bg.gamma_multiply(0.38))
                                                    .stroke(Stroke::new(1.0, palette.border))
                                                    .corner_radius(8)
                                                    .inner_margin(Margin::symmetric(14, 9))
                                            } else if block_is_quote {
                                                egui::Frame::new()
                                                    .fill(palette.accent_soft.gamma_multiply(0.42))
                                                    .stroke(Stroke::new(1.0, palette.border))
                                                    .corner_radius(6)
                                                    .inner_margin(Margin::symmetric(14, 9))
                                            } else {
                                                egui::Frame::new()
                                            };
                                            let mut pending_scroll_rect = None;
                                            if !block_is_code {
                                                ui.add_space(6.0);
                                            }
                                            let editor_frame = frame.show(ui, |ui| {
                                                if !block_is_code {
                                                    ui.set_min_height(WYSIWYG_BODY_LINE_HEIGHT);
                                                }
                                                if block_is_code {
                                                    show_code_header(ui, code_language.as_deref(), palette);
                                                }
                                                let desired_rows = 1;
                                                let input_action = editor_input_action(ui);
                                                let mut layouter =
                                                    |ui: &Ui,
                                                     buffer: &dyn egui::TextBuffer,
                                                     wrap_width: f32| {
                                                        if block_is_code {
                                                            syntax_highlighted_code_layout(
                                                                ui,
                                                                buffer.as_str(),
                                                                wrap_width,
                                                                palette,
                                                                code_language.as_deref(),
                                                            )
                                                        } else {
                                                            wysiwyg_layout(
                                                                ui,
                                                                buffer.as_str(),
                                                                &projection,
                                                                wrap_width,
                                                                palette,
                                                                true,
                                                            )
                                                        }
                                                    };
                                                let mut editor =
                                                    TextEdit::multiline(&mut visual_content)
                                                        .id(editor_id)
                                                        .frame(egui::Frame::NONE)
                                                        .layouter(&mut layouter)
                                                        .hint_text(if block_is_code {
                                                            "输入代码…"
                                                        } else if source.is_empty() {
                                                            "写下第一行…"
                                                        } else {
                                                            ""
                                                        })
                                                        .desired_width(f32::INFINITY)
                                                        .desired_rows(desired_rows)
                                                        .lock_focus(true);
                                                if block_is_code {
                                                    editor = editor.code_editor();
                                                }
                                                let inline_code_background = (!block_is_code)
                                                    .then(|| ui.painter().add(egui::Shape::Noop));
                                                let mut output = editor.show(ui);
                                                if original_block.is_empty() {
                                                    let rect = egui::Rect::from_min_size(output.response.rect.min,
                                                        Vec2::new(output.response.rect.width(), WYSIWYG_BODY_LINE_HEIGHT));
                                                    if ui.interact(rect, ui.make_persistent_id(("empty-paragraph-focus", block.id)), egui::Sense::click()).clicked() {
                                                        ui.memory_mut(|memory| memory.request_focus(editor_id));
                                                    }
                                                }
                                                pointer_regions.push(HybridPointerRegion {
                                                    block_id: block.id,
                                                    source_range: edit_range.clone(),
                                                    rect: output.response.rect,
                                                    atomic_range: block_is_code
                                                        .then(|| block.range.clone()),
                                                    mapping: NativePointerMapping::Text(
                                                        TextProjectionPreview {
                                                            rect: output.response.rect,
                                                            galley: Arc::clone(&output.galley),
                                                            galley_pos: output.galley_pos,
                                                            projection: projection.clone(),
                                                        },
                                                    ),
                                                });
                                                if output.response.clicked()
                                                    && let Some(position) =
                                                        output.response.interact_pointer_pos()
                                                {
                                                    let visual_cursor = usize::from(
                                                        text_edit_cursor_at_position(
                                                            &output, position,
                                                        )
                                                        .index,
                                                    );
                                                    let source_char = projection
                                                        .source_char_range(
                                                            &original_block,
                                                            visual_cursor..visual_cursor,
                                                        )
                                                        .start;
                                                    let source_byte = char_to_byte(
                                                        &original_block,
                                                        source_char,
                                                    );
                                                    if !block_is_code
                                                        && let Some(updated) = markdown::toggle_task_marker_at(
                                                            &original_block, source_byte,
                                                        )
                                                    {
                                                        pending_edit = Some((edit_range.clone(), updated, EditKind::TaskList));
                                                    } else if ui.input(|input| input.modifiers.command)
                                                        && let Some(destination) =
                                                        markdown::link_destination_with_references(
                                                            &original_block,
                                                            source_byte,
                                                            &references,
                                                        )
                                                    {
                                                        clicked_destination = Some(destination);
                                                    }
                                                }
                                                if let Some(shape_index) = inline_code_background {
                                                    let runs = projection.runs_for(&visual_content);
                                                    ui.painter().set(
                                                        shape_index,
                                                        egui::Shape::Vec(
                                                            rounded_inline_code_backgrounds(
                                                                &output.galley,
                                                                output.galley_pos,
                                                                &runs,
                                                                palette,
                                                            ),
                                                        ),
                                                    );
                                                    paint_inline_code_delimiters(
                                                        ui,
                                                        &output.galley,
                                                        output.galley_pos,
                                                        &runs,
                                                        palette,
                                                    );
                                                }
                                                if output.response.has_focus() {
                                                    let accessible_remainder =
                                                        accessible_document_remainder(
                                                            source, blocks, block.id,
                                                        );
                                                    append_accessible_text_runs(
                                                        ui,
                                                        editor_id,
                                                        (document_id, block.id),
                                                        &accessible_remainder,
                                                        output.response.rect,
                                                    );
                                                }
                                                set_accessible_label(
                                                    ui.ctx(),
                                                    editor_id,
                                                    format!(
                                                        "从第 {} 行开始的所见即所得编辑块",
                                                        block.line
                                                    ),
                                                );
                                                let visual_cursor_after =
                                                    text_edit_cursor_after_input(&output);
                                                if pending_local_cursor.is_some()
                                                    && let Some(cursor) = visual_cursor_after
                                                {
                                                    let caret = output.galley.pos_from_cursor(cursor.primary)
                                                        .translate(output.galley_pos.to_vec2());
                                                    pending_scroll_rect = Some(caret);
                                                }
                                                let mut visual_selection_after =
                                                    visual_cursor_after
                                                        .map(cursor_range_to_char_range);
                                                if output.response.dragged()
                                                    && let Some(anchor_position) = ui
                                                        .input(|input| input.pointer.press_origin())
                                                {
                                                    // `TextEditOutput::cursor_range` is captured
                                                    // before pointer interaction in egui 0.35. On
                                                    // Windows a right-to-left drag can therefore be
                                                    // observed as only the moving caret. Rebuild the
                                                    // directed range from the stable press origin
                                                    // and current pointer position using the exact
                                                    // galley hit-test used by TextEdit itself.
                                                    let anchor = text_edit_cursor_at_position(
                                                        &output,
                                                        anchor_position,
                                                    );
                                                    let current_position = ui
                                                        .input(|input| input.pointer.interact_pos())
                                                        .unwrap_or(anchor_position);
                                                    let current = text_edit_cursor_at_position(
                                                        &output,
                                                        current_position,
                                                    );
                                                    let cursor = CCursorRange {
                                                        primary: current,
                                                        secondary: anchor,
                                                        h_pos: None,
                                                    };
                                                    visual_selection_after =
                                                        Some(cursor_range_to_char_range(cursor));
                                                    output
                                                        .state
                                                        .cursor
                                                        .set_char_range(Some(cursor));
                                                    output
                                                        .state
                                                        .clone()
                                                        .store(ui.ctx(), output.response.id);
                                                }
                                                let focused = output.response.has_focus();
                                                // TextEdit also marks boundary Backspace/Delete as
                                                // changed when the buffer stays identical. Those
                                                // keys still need the cross-block boundary handler.
                                                let mut changed = output.response.changed()
                                                    && visual_content != projection.text();
                                                let mut kind = EditKind::Typing;
                                                let mut source_update = None;
                                                let mut boundary_input_handled = false;
                                                let defer_ime = focused
                                                    && (ime_action == ImeFrameAction::Preedit
                                                        || had_ime_session
                                                            && ime_action == ImeFrameAction::None);

                                                if defer_ime {
                                                    changed = false;
                                                } else if had_ime_session
                                                    && ime_action == ImeFrameAction::Cancel
                                                    && !ime_has_commit
                                                {
                                                    ime_session = None;
                                                    changed = false;
                                                } else if had_ime_session
                                                    && ime_action == ImeFrameAction::Commit
                                                {
                                                    ime_session = None;
                                                } else if focused
                                                    && let Some(selection) = local_source_selection_before.as_ref()
                                                    && let Some(cursor) = fenced_boundary_delete_cursor(
                                                        source, blocks, block_index,
                                                        selection.clone(), input_action.backspace, input_action.delete,
                                                    )
                                                {
                                                    // Keep the line break required to recognize a fence.
                                                    // Crossing this boundary only moves the caret.
                                                    next_global_cursor = Some(CCursorRange::one(CCursor::new(cursor)));
                                                    cursor_adjusted = true;
                                                    boundary_input_handled = true;
                                                    changed = false;
                                                } else if !defer_ime
                                                    && focused
                                                    && !changed
                                                    && !input_action.horizontal_modified
                                                    && let Some(selection) =
                                                        local_source_selection_before.clone()
                                                    && let Some(selection) =
                                                        move_across_hidden_inline_code_boundary(
                                                            &original_block,
                                                            selection,
                                                            input_action.left,
                                                            input_action.right,
                                                        )
                                                {
                                                    next_global_cursor = Some(CCursorRange::two(
                                                        CCursor::new(
                                                            block_char_start + selection.start,
                                                        ),
                                                        CCursor::new(
                                                            block_char_start + selection.end,
                                                        ),
                                                    ));
                                                    cursor_adjusted = true;
                                                    boundary_input_handled = true;
                                                }

                                                if !boundary_input_handled
                                                    && !defer_ime
                                                    && focused
                                                    && !block_is_code
                                                    && let (Some(url), Some(selection)) = (
                                                        input_action.pasted_text.as_deref(),
                                                        visual_selection_before.clone(),
                                                    )
                                                    && !selection.is_empty()
                                                    && selection_before.as_ref().is_some_and(|selection| {
                                                        !selection_intersects_code(source, selection)
                                                    })
                                                {
                                                    let source_selection = projection
                                                        .source_char_range(
                                                            &original_block,
                                                            selection,
                                                        );
                                                    let mut updated = original_block.clone();
                                                    if let Some(next) =
                                                        editing::paste_url_as_markdown_link(
                                                            &mut updated,
                                                            source_selection,
                                                            url,
                                                        )
                                                    {
                                                        source_update = Some((updated, next));
                                                        kind = EditKind::Other;
                                                        changed = true;
                                                        cursor_adjusted = true;
                                                    }
                                                } else if !defer_ime && focused && input_action.tab
                                                {
                                                    let selection = visual_selection_before
                                                        .clone()
                                                        .unwrap_or_else(|| {
                                                            let end =
                                                                visual_content.chars().count();
                                                            end..end
                                                        });
                                                    let source_selection = projection
                                                        .source_char_range(
                                                            &original_block,
                                                            selection,
                                                        );
                                                    let mut updated = original_block.clone();
                                                    let next = editing::indent_selected_lines(
                                                        &mut updated,
                                                        source_selection,
                                                        input_action.shift,
                                                    );
                                                    source_update = Some((updated, next));
                                                    kind = EditKind::Other;
                                                    changed = true;
                                                    cursor_adjusted = true;
                                                } else if !defer_ime
                                                    && focused
                                                    && let (Some(typed), Some(selection)) = (
                                                        input_action.typed_text.as_deref(),
                                                        visual_selection_before.clone(),
                                                    )
                                                {
                                                    let mut paired = projection.text().to_owned();
                                                    if let Some(next) = editing::apply_smart_pair(
                                                        &mut paired,
                                                        selection,
                                                        typed,
                                                    ) {
                                                        changed = paired != projection.text();
                                                        visual_content = paired;
                                                        visual_selection_after = Some(next);
                                                        kind = EditKind::Other;
                                                        cursor_adjusted = true;
                                                    }
                                                } else if !defer_ime
                                                    && focused
                                                    && block_is_code
                                                    && (input_action.backspace
                                                        || input_action.delete)
                                                    && !changed
                                                    && projection.text().trim().is_empty()
                                                {
                                                    let replacement_range =
                                                        code_block_removal_range(
                                                            source,
                                                            blocks,
                                                            block_index,
                                                        );
                                                    let cursor = source[..replacement_range.start]
                                                        .chars()
                                                        .count();
                                                    pending_edit = Some((
                                                        replacement_range,
                                                        String::new(),
                                                        EditKind::Typing,
                                                    ));
                                                    next_global_cursor = Some(CCursorRange::one(
                                                        CCursor::new(cursor),
                                                    ));
                                                    cursor_adjusted = true;
                                                    boundary_input_handled = true;
                                                } else if !defer_ime
                                                    && focused
                                                    && input_action.backspace
                                                    && !boundary_input_handled
                                                    && !changed
                                                    && visual_selection_before.as_ref().is_some_and(
                                                        |selection| {
                                                            selection.is_empty()
                                                                && selection.start == 0
                                                        },
                                                    )
                                                    && let Some((replacement_range, cursor)) =
                                                        boundary_backspace_edit(
                                                            source,
                                                            edit_range.clone(),
                                                        )
                                                {
                                                    pending_edit = Some((
                                                        replacement_range,
                                                        original_block.clone(),
                                                        EditKind::Typing,
                                                    ));
                                                    next_global_cursor = Some(CCursorRange::one(
                                                        CCursor::new(cursor),
                                                    ));
                                                    cursor_adjusted = true;
                                                    boundary_input_handled = true;
                                                }

                                                if !boundary_input_handled
                                                    && !defer_ime
                                                    && source_update.is_none()
                                                    && changed
                                                    && let Some(selection) =
                                                        visual_selection_after.clone()
                                                    && let Some(mut update) = projection.apply_edit(
                                                        &original_block,
                                                        &visual_content,
                                                        selection,
                                                    )
                                                {
                                                    if focused && input_action.enter {
                                                        if !input_action.shift
                                                            && let Some(selection) =
                                                                complete_fenced_code_on_enter(
                                                                    &mut update.source,
                                                                    update.selection.clone(),
                                                                )
                                                        {
                                                            update.selection = selection;
                                                        } else if !block_is_code {
                                                            update.selection =
                                                                complete_visual_enter(
                                                                    &mut update.source,
                                                                    update.selection,
                                                                    input_action.shift,
                                                                );
                                                        }
                                                    }
                                                    if let Some(selection) =
                                                        consume_paired_fenced_code_closer(
                                                            &mut update.source,
                                                            update.selection.clone(),
                                                        )
                                                    {
                                                        update.selection = selection;
                                                    } else if let Some(selection) =
                                                        complete_bare_fenced_code_after_typing(
                                                            &mut update.source,
                                                            update.selection.clone(),
                                                        )
                                                    {
                                                        update.selection = selection;
                                                    }
                                                    cursor_adjusted = true;
                                                    source_update =
                                                        Some((update.source, update.selection));
                                                }

                                                if !boundary_input_handled
                                                    && !defer_ime
                                                    && source_update.is_none()
                                                    && focused
                                                    && block_is_code
                                                    && !input_action.horizontal_modified
                                                    && (input_action.right || input_action.down)
                                                    && visual_selection_before.as_ref().is_some_and(
                                                        |selection| {
                                                            selection.is_empty()
                                                                && selection.end
                                                                    == projection
                                                                        .text()
                                                                        .chars()
                                                                        .count()
                                                        },
                                                    )
                                                {
                                                    // Arrow keys only navigate. Creating a paragraph
                                                    // here changes Markdown and leaves the cursor in
                                                    // the hidden separator after the closing fence.
                                                    if let Some(next) = blocks.get(block_index + 1)
                                                    {
                                                        let cursor = source[..next.range.start]
                                                            .chars()
                                                            .count();
                                                        next_global_cursor = Some(CCursorRange::one(
                                                            CCursor::new(cursor),
                                                        ));
                                                        cursor_adjusted = true;
                                                        boundary_input_handled = true;
                                                    }
                                                }

                                                if defer_ime {
                                                    // IME pre-edit text belongs to the composition,
                                                    // not to the Markdown document or its undo history.
                                                    // Keeping this visual buffer alive lets egui replace
                                                    // the previous pre-edit range on the next frame.
                                                    let mut base_source = original_block;
                                                    if ime_has_commit && let Some(composition) = visual_selection_after {
                                                        // A committed composition and the next pre-edit
                                                        // can arrive together. Persist everything except
                                                        // the currently selected pre-edit range.
                                                        let mut committed = visual_content.clone();
                                                        let range = char_to_byte(&committed, composition.start)..char_to_byte(&committed, composition.end);
                                                        committed.replace_range(range, "");
                                                        if let Some(update) = projection.apply_edit(&base_source, &committed, composition.start..composition.start) {
                                                            let cursor = CCursorRange::one(CCursor::new(block_char_start + update.selection.end));
                                                            next_global_cursor = Some(cursor);
                                                            base_source = update.source;
                                                            pending_edit = Some((edit_range.clone(), base_source.clone(), EditKind::Typing));
                                                        }
                                                    }
                                                    ime_session = Some(HybridImeSession {
                                                        document_id,
                                                        block_id: block.id,
                                                        base_source,
                                                        visual_content,
                                                    });
                                                } else if !boundary_input_handled
                                                    && let Some((mut updated, mut selection)) =
                                                        source_update
                                                {
                                                    if block.range.is_empty() && !updated.trim().is_empty() {
                                                        let (prefix, suffix) = empty_paragraph_separators(source, blocks, block_index);
                                                        if prefix > 0 { updated.insert_str(0, &"\n".repeat(prefix)); }
                                                        updated.push_str(&"\n".repeat(suffix));
                                                        selection = selection.start + prefix..selection.end + prefix;
                                                    }
                                                    next_global_cursor = Some(CCursorRange::two(
                                                        CCursor::new(
                                                            block_char_start + selection.start,
                                                        ),
                                                        CCursor::new(
                                                            block_char_start + selection.end,
                                                        ),
                                                    ));
                                                    pending_edit =
                                                        Some((edit_range.clone(), updated, kind));
                                                } else if !boundary_input_handled
                                                    && let Some(selection) = visual_selection_after
                                                {
                                                    let source_selection =
                                                        source_selection_after_visual_input(
                                                            &projection,
                                                            &original_block,
                                                            local_source_selection_before.as_ref(),
                                                            visual_selection_before.as_ref(),
                                                            selection,
                                                        );
                                                    let local_cursor = if let Some(visual_cursor) =
                                                        visual_cursor_after
                                                    {
                                                        cursor_range_with_direction(
                                                            source_selection,
                                                            visual_cursor,
                                                        )
                                                    } else {
                                                        CCursorRange::two(
                                                            CCursor::new(source_selection.start),
                                                            CCursor::new(source_selection.end),
                                                        )
                                                    };
                                                    next_global_cursor = Some(cursor_range_add(
                                                        local_cursor,
                                                        block_char_start,
                                                    ));
                                                }
                                            });
                                            if !block_is_code {
                                                ui.add_space(6.0);
                                            }
                                            active_editor_rect = Some(editor_frame.response.rect);
                                            if pending_local_cursor.is_some() {
                                                let target = pending_scroll_rect.unwrap_or(editor_frame.response.rect);
                                                ui.scroll_to_rect(target, None);
                                                // Nearby virtualized blocks gain their measured
                                                // heights during the jump. Keep following the caret
                                                // until it is actually inside the viewport.
                                                if !ui.clip_rect().contains_rect(target) {
                                                    cursor_adjusted = true;
                                                }
                                            }
                                            if block_is_code {
                                                code_surface_rect =
                                                    Some(editor_frame.response.rect);
                                                if let Some(region) = pointer_regions.last_mut()
                                                    && region.block_id == block.id
                                                {
                                                    region.rect = editor_frame.response.rect;
                                                }
                                            }
                                        } else {
                                            if !block_is_code {
                                                ui.add_space(6.0);
                                            }
                                            let preview = prepared_block.show(ui);
                                            if !block_is_code {
                                                ui.add_space(6.0);
                                            }
                                            let activation_id =
                                                ui.make_persistent_id(("activate-block", block.id));
                                            let response = ui
                                                .interact(
                                                    preview.rect,
                                                    activation_id,
                                                    egui::Sense::click_and_drag(),
                                                )
                                                .on_hover_text(format!(
                                                    "点击编辑第 {} 行开始的 Markdown 块；Ctrl+点击打开链接",
                                                    block.line
                                                ));
                                            set_markdown_preview_accessibility(
                                                &response,
                                                preview_source,
                                            );
                                            pointer_regions.push(HybridPointerRegion {
                                                block_id: block.id,
                                                source_range: preview_range.clone(),
                                                rect: preview.rect,
                                                atomic_range: (block_is_code
                                                    || preview.atomic
                                                    || generated_preview)
                                                .then(|| block.range.clone()),
                                                mapping: if generated_preview {
                                                    NativePointerMapping::Atomic { source_range: 0..source_block.len() }
                                                } else {
                                                    preview.mapping.clone()
                                                },
                                            });
                                            if response.clicked() {
                                                let local_source_byte = response
                                                    .interact_pointer_pos()
                                                    .map(|position| {
                                                        preview.mapping.source_byte_at_position(
                                                            preview_source,
                                                            preview.rect,
                                                            position,
                                                        )
                                                    })
                                                    .unwrap_or_default();
                                                let command_click =
                                                    ui.input(|input| input.modifiers.command);
                                                if !generated_preview
                                                    && let Some(updated) =
                                                        markdown::toggle_task_marker_at(
                                                            source_block,
                                                            local_source_byte,
                                                        )
                                                {
                                                    pending_edit = Some((
                                                        block.range.clone(),
                                                        updated,
                                                        EditKind::TaskList,
                                                    ));
                                                } else if command_click
                                                    && let Some(destination) =
                                                        markdown::link_destination_with_references(
                                                            preview_source,
                                                            local_source_byte,
                                                            &references,
                                                        )
                                                {
                                                    clicked_destination = Some(destination);
                                                } else {
                                                    activate = Some((
                                                        block.id,
                                                        if generated_preview {
                                                            block.range.start
                                                        } else {
                                                            block.range.start + local_source_byte
                                                        },
                                                    ));
                                                }
                                            }
                                            if block_is_code {
                                                code_surface_rect = Some(preview.rect);
                                            }
                                        }
                                    });
                                    if let (Some(rect), Some(content)) =
                                        (code_surface_rect, code_content)
                                        && show_code_copy_button(
                                            ui,
                                            (document_id, block.id),
                                            rect,
                                            content,
                                            palette,
                                        )
                                    {
                                        copied_code_block = true;
                                    }

                                    if block_is_code {
                                        let response = ui
                                            .allocate_response(
                                                Vec2::new(ui.available_width(), 24.0),
                                                egui::Sense::click(),
                                            )
                                            .on_hover_cursor(egui::CursorIcon::Text)
                                            .on_hover_text("双击在代码块后新建普通段落");
                                        if response.hovered() {
                                            ui.painter().text(
                                                response.rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "双击新建段落",
                                                FontId::proportional(11.0),
                                                palette.secondary.gamma_multiply(0.75),
                                            );
                                        }
                                        if response.double_clicked() && pending_edit.is_none() {
                                            let (tail_range, replacement, cursor) =
                                                paragraph_after_code_double_click(
                                                    source,
                                                    blocks,
                                                    block_index,
                                                );
                                            next_global_cursor =
                                                Some(CCursorRange::one(CCursor::new(cursor)));
                                            if let Some(replacement) = replacement {
                                                pending_edit = Some((
                                                    tail_range,
                                                    replacement,
                                                    EditKind::Typing,
                                                ));
                                            }
                                            cursor_adjusted = true;
                                        }
                                    } else {
                                        ui.add_space(8.0);
                                    }
                                    let measured_height =
                                        (ui.next_widget_position().y - block_layout_start).max(1.0);
                                    // Editing reveals cursor-dependent Markdown markers. Its
                                    // measured height is not an inactive preview measurement.
                                    if Some(block.id) != active_id {
                                        prepared_block.remember_height(measured_height);
                                    }
                                }
                                ui.add_space(60.0);
                            });
                        });
                    page_rect = Some(page.response.rect);
                });
                ui.add_space(40.0);
            });

        if copied_code_block {
            effects.notice = "已复制代码块".to_owned();
        }
        if let Some(destination) = clicked_destination {
            effects.destination = Some(destination);
        }

        let pointer = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_released(),
                input.pointer.primary_down(),
                input.pointer.is_decidedly_dragging(),
                input.pointer.press_origin(),
                input.pointer.interact_pos(),
            )
        });
        if pointer.0
            && let Some(origin) = pointer.4
            && let Some((block_id, source_char)) =
                hybrid_pointer_hit(source, &pointer_regions, origin)
        {
            self.hybrid_pointer_anchor = Some(HybridPointerAnchor {
                document_id,
                block_id,
                source_char,
            });
            self.hybrid_cross_selection = None;
        }
        if pointer.2
            && pointer.3
            && let (Some(anchor), Some(position)) = (self.hybrid_pointer_anchor, pointer.5)
            && anchor.document_id == document_id
            && let Some((current_block, current_char)) =
                hybrid_pointer_hit(source, &pointer_regions, position)
        {
            if current_block != anchor.block_id {
                let cursor = snap_atomic_cross_block_selection(
                    source,
                    blocks,
                    anchor.block_id,
                    anchor.source_char,
                    current_block,
                    current_char,
                );
                self.hybrid_cross_selection = Some(HybridCrossSelection {
                    document_id,
                    cursor,
                });
                next_global_cursor = Some(cursor);
            } else if Some(current_block) != active_id
                && anchor.source_char != current_char
                && let Some(atomic) = pointer_regions
                    .iter()
                    .find(|region| region.block_id == current_block)
                    .and_then(|region| region.atomic_range.clone())
            {
                let start = source[..atomic.start].chars().count();
                let end = source[..atomic.end].chars().count();
                let cursor = CCursorRange::two(CCursor::new(start), CCursor::new(end));
                self.hybrid_cross_selection = Some(HybridCrossSelection {
                    document_id,
                    cursor,
                });
                next_global_cursor = Some(cursor);
            } else {
                self.hybrid_cross_selection = None;
            }
        }
        if pointer.1 {
            self.hybrid_pointer_anchor = None;
        }
        if let Some(selection) = self
            .hybrid_cross_selection
            .filter(|selection| selection.document_id == document_id)
        {
            next_global_cursor = Some(selection.cursor);
            paint_hybrid_cross_selection(ui, source, &pointer_regions, selection.cursor);
        }

        let deactivate = ui.input(|input| {
            input.key_pressed(Key::Escape)
                || input.pointer.any_click()
                    && input.pointer.interact_pos().is_some_and(|position| {
                        page_rect.is_some_and(|rect| rect.contains(position))
                            && active_editor_rect.is_some_and(|rect| !rect.contains(position))
                    })
        });

        if let Some(cursor_range) = next_global_cursor {
            self.editor_cursor = Some(cursor_range);
        }
        // Resolve activation while the rendered source is still borrowed. An
        // edit below can change its byte offsets before activation is applied.
        let activate = activate.map(|(id, start)| (id, source[..start].chars().count()));
        if let Some((range, replacement, kind)) = pending_edit {
            let selection_after = next_global_cursor.map(cursor_range_to_char_range);
            document.edit(kind, selection_before, |content| {
                content.replace_range(range, &replacement);
                selection_after
            });
            let cursor_block_id = next_global_cursor.map(|cursor_range| {
                let cursor = cursor_range.sorted_cursors()[1].index.0;
                let updated = document.render_view();
                block_for_char_index(updated.source, updated.blocks, cursor).id
            });
            if let Some(next_active_id) = cursor_block_id.or(active_id) {
                self.hybrid_active = Some((document_id, next_active_id));
                if cursor_block_id != active_id {
                    self.pending_editor_cursor = next_global_cursor;
                }
            }
            if !copied_code_block {
                effects.notice = if kind == EditKind::TaskList {
                    "已更新任务列表"
                } else {
                    "已更新当前 Markdown 块"
                }
                .to_owned();
            }
        }
        if cursor_adjusted {
            self.pending_editor_cursor = next_global_cursor;
        }
        if deactivate && self.hybrid_cross_selection.is_none() {
            self.hybrid_active = None;
            ime_session = None;
        }
        if let Some((id, char_start)) = activate {
            self.hybrid_active = Some((document_id, id));
            ime_session = None;
            self.queue_editor_selection(char_start..char_start);
        }
        self.hybrid_ime_session = ime_session;
        self.finish_history_action(document, effects);
    }
}
