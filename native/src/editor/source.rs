//! Source text editing and context commands.
use super::*;

impl EditorSurface {
    pub(super) fn edit_pane(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        options: &EditorOptions<'_>,
        effects: &mut EditorOutput,
        scroll_offset: Option<f32>,
    ) -> PaneScroll {
        normalize_editor_input_line_endings(ui);
        self.finish_leading_ime_cancel(ui, document);
        if self.source_ime_session.is_some()
            && ui.input(|input| {
                input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Cut
                            | egui::Event::Key { pressed: true, .. }
                            | egui::Event::PointerButton { pressed: true, .. }
                    )
                })
            })
        {
            // egui-winit filters keys consumed by the system IME. Ordinary
            // input that reaches the editor interrupts composition: restore
            // its durable selection before TextEdit executes the new action.
            self.source_ime_session = None;
            if let Some(editor_id) = self.editor_widget_id {
                let mut state = egui::text_edit::TextEditState::default();
                state.cursor.set_char_range(self.editor_cursor);
                state.store(ui.ctx(), editor_id);
            }
            self.ime_interrupted_frame = Some(ui.ctx().cumulative_frame_nr());
        }
        if take_leading_select_all(ui, self.editor_widget_id) {
            self.queue_editor_selection(0..document.content.chars().count());
        }
        if self.apply_leading_tab_input(ui, document) {
            effects.notice = "已修改缩进".to_owned();
        }
        let cursor_before = self.editor_cursor;
        let mut selection_before = cursor_before.map(cursor_range_to_char_range);
        let mut scroll_area = ScrollArea::vertical().id_salt(("editor-scroll", document.id()));
        if let Some(offset) = scroll_offset {
            scroll_area = scroll_area.vertical_scroll_offset(offset);
        }
        let mut context_command = None;
        let palette = app_palette(options.dark);
        let viewport_height = ui.available_height();
        let output = scroll_area.show(ui, |ui| {
            let page_layout = document_page_layout(ui.available_width(), viewport_height);
            ui.add_space(page_layout.top_margin);
            ui.horizontal(|ui| {
                ui.add_space(page_layout.side_margin);
                document_page_frame(palette, options.dark, &page_layout).show(ui, |ui| {
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.set_width(page_layout.content_width);
                        ui.set_min_height(page_layout.min_content_height);
                        let available =
                            Vec2::new(ui.available_width(), page_layout.min_content_height);
                        let row_height = ui.text_style_height(&egui::TextStyle::Monospace).max(1.0);
                        let desired_rows = (available.y / row_height).max(1.0) as usize;
                        let editor_id = ui.make_persistent_id(("editor", document.id()));
                        self.editor_widget_id = Some(editor_id);
                        let requested_cursor = self.pending_editor_cursor;
                        if let Some(cursor_range) = self.pending_editor_cursor.take() {
                            // A new source selection also ends any previous
                            // TextEdit composition after view/document changes.
                            let mut state = if self.source_ime_session.is_some() {
                                TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default()
                            } else {
                                egui::text_edit::TextEditState::default()
                            };
                            state.cursor.set_char_range(Some(cursor_range));
                            state.store(ui.ctx(), editor_id);
                            ui.memory_mut(|memory| memory.request_focus(editor_id));
                            self.editor_cursor = Some(cursor_range);
                        }
                        let input_action = editor_input_action(ui);
                        let (ime_action, ime_has_commit, starts_preedit) = ui.input(|input| {
                            (
                                ime_frame_action(&input.events),
                                input.events.iter().any(|event| matches!(event,
                                    egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty())),
                                input.events.iter().any(|event| matches!(event,
                                    egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) if !text.is_empty())),
                            )
                        });
                        let mut composition = self.source_ime_session.take()
                            .filter(|session| session.document_id == document.id());
                        if composition.is_none() && starts_preedit
                            && ui.memory(|memory| memory.has_focus(editor_id))
                        {
                            // A later accepted composition supersedes an
                            // earlier interruption within this native frame.
                            if ui.input(|input| input.focused) {
                                self.ime_interrupted_frame = None;
                            }
                            // TextEdit's internal focus survives window focus
                            // loss and can consume a late preedit. Keep that
                            // spelling out of the document even while unfocused.
                            composition = Some(SourceImeSession {
                                document_id: document.id(),
                                visual_content: document.content.clone(),
                                selection_before: selection_before.clone(),
                            });
                        }
                        let mut editor_buffer = TrackingTextBuffer::new(
                            if let Some(session) = composition.as_mut() {
                                &mut session.visual_content
                            } else {
                                &mut document.content
                            },
                        );
                        let output = TextEdit::multiline(&mut editor_buffer)
                            .id(editor_id)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .hint_text("写下第一行…")
                            .desired_width(f32::INFINITY)
                            .desired_rows(desired_rows)
                            .lock_focus(true)
                            .show(ui);
                        if let Some(cursor) = requested_cursor {
                            let cursor = text_edit_cursor_after_input(&output).unwrap_or(cursor);
                            let caret = output
                                .galley
                                .pos_from_cursor(cursor.primary)
                                .translate(output.galley_pos.to_vec2());
                            ui.scroll_to_rect(caret, None);
                        }
                        set_accessible_label(ui.ctx(), editor_id, "Markdown 源码编辑区");
                        let mut before_content = editor_buffer.take_before();
                        drop(editor_buffer);
                        output.response.context_menu(|ui| {
                            if ui.button("撤销").clicked() {
                                context_command = Some(ContextCommand::Undo);
                                ui.close();
                            }
                            if ui.button("重做").clicked() {
                                context_command = Some(ContextCommand::Redo);
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("粗体").clicked() {
                                context_command =
                                    Some(ContextCommand::Format(MarkdownCommand::Bold));
                                ui.close();
                            }
                            if ui.button("斜体").clicked() {
                                context_command =
                                    Some(ContextCommand::Format(MarkdownCommand::Italic));
                                ui.close();
                            }
                            if ui.button("链接").clicked() {
                                context_command =
                                    Some(ContextCommand::Format(MarkdownCommand::Link));
                                ui.close();
                            }
                            if ui.button("行内代码").clicked() {
                                context_command =
                                    Some(ContextCommand::Format(MarkdownCommand::InlineCode));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("粘贴剪贴板图片").clicked() {
                                context_command = Some(ContextCommand::PasteImage);
                                ui.close();
                            }
                            if ui.button("可视化编辑表格").clicked() {
                                context_command = Some(ContextCommand::EditTable);
                                ui.close();
                            }
                        });
                        let cursor_after = text_edit_cursor_after_input(&output);
                        let mut selection_after = cursor_after.map(cursor_range_to_char_range);
                        if let Some(session) = composition {
                            let defer = output.response.has_focus() && ui.input(|input| input.focused)
                                && matches!(ime_action, ImeFrameAction::Preedit | ImeFrameAction::None);
                            if defer {
                                self.source_ime_session = Some(session);
                                return;
                            }
                            if !ime_has_commit {
                                // Cancellation or loss of focus must not turn
                                // the displayed spelling into durable content.
                                let mut state = egui::text_edit::TextEditState::default();
                                state.cursor.set_char_range(self.editor_cursor);
                                state.store(ui.ctx(), editor_id);
                                return;
                            }
                            selection_before = session.selection_before;
                            before_content = Some(std::mem::replace(
                                &mut document.content, session.visual_content,
                            ));
                        }
                        if let Some(cursor_range) = cursor_after {
                            self.editor_cursor = Some(cursor_range);
                        }
                        // egui can surrender focus after accepting Tab. Finish
                        // the smart edit for input this widget already consumed.
                        let accepted_input =
                            output.response.has_focus() || output.response.changed();
                        let mut changed = output.response.changed();
                        // A composition is a single history transaction,
                        // independent of typing pauses or earlier plain input.
                        let mut kind = if ime_has_commit { EditKind::Other } else { EditKind::Typing };
                        let mut cursor_adjusted = false;
                        if accepted_input
                            && let (Some(url), Some(selection)) = (
                                input_action.pasted_text.as_deref(),
                                selection_before.clone(),
                            )
                            && !selection.is_empty()
                            && let Some(before_content) = before_content.as_ref()
                            && !selection_intersects_code(before_content, &selection)
                        {
                            let mut linked = before_content.clone();
                            if let Some(next) =
                                editing::paste_url_as_markdown_link(&mut linked, selection, url)
                            {
                                document.content = linked;
                                selection_after = Some(next);
                                kind = EditKind::Other;
                                changed = true;
                                cursor_adjusted = true;
                            }
                        } else if accepted_input
                            && input_action.tab
                            && let Some(before_content) = before_content.as_ref()
                        {
                            document.content.clone_from(before_content);
                            let selection = selection_before.clone().unwrap_or_else(|| {
                                let end = before_content.chars().count();
                                end..end
                            });
                            selection_after = Some(editing::indent_selected_lines(
                                &mut document.content,
                                selection,
                                input_action.shift,
                            ));
                            kind = EditKind::Other;
                            changed = true;
                            cursor_adjusted = true;
                        } else if accepted_input
                            && let (Some(typed), Some(selection)) =
                                (input_action.typed_text.as_deref(), selection_before.clone())
                            && let Some(before_content) = before_content.as_ref()
                        {
                            let mut paired = before_content.clone();
                            if let Some(next) =
                                editing::apply_smart_pair(&mut paired, selection, typed)
                            {
                                changed = paired != *before_content;
                                document.content = paired;
                                selection_after = Some(next);
                                kind = EditKind::Other;
                                cursor_adjusted = true;
                            }
                        } else if accepted_input
                            && changed
                            && input_action.enter
                            && let Some(cursor) = selection_after.as_ref().map(|range| range.end)
                            && let Some(next) =
                                editing::continue_markdown_line(&mut document.content, cursor)
                        {
                            selection_after = Some(next);
                            cursor_adjusted = true;
                        }

                        if changed
                            && let Some(before_content) = before_content.take()
                            && document.record_edit(
                                before_content,
                                selection_before,
                                selection_after.clone(),
                                kind,
                            )
                        {
                            effects.notice = "已修改".to_owned();
                        }
                        if cursor_adjusted && let Some(selection) = selection_after {
                            let direction =
                                cursor_before.unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
                            self.select_cursor(cursor_range_with_direction(selection, direction));
                        }
                    });
                });
            });
            ui.add_space(32.0);
        });
        if let Some(command) = context_command {
            self.apply_context_command(document, command, effects);
        }
        self.finish_deferred_command(document, effects);
        PaneScroll {
            offset: output.state.offset.y,
            maximum: (output.content_size.y - output.inner_rect.height()).max(0.0),
            hovered: ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|position| output.inner_rect.contains(position)),
        }
    }
}
