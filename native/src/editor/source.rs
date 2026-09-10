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
        if take_leading_select_all(ui, self.editor_widget_id) {
            self.queue_editor_selection(0..document.content.chars().count());
        }
        if self.apply_leading_tab_input(ui, document) {
            effects.notice = "已修改缩进".to_owned();
        }
        let cursor_before = self.editor_cursor;
        let selection_before = cursor_before.map(cursor_range_to_char_range);
        let mut scroll_area = ScrollArea::vertical().id_salt(("editor-scroll", document.id()));
        if let Some(offset) = scroll_offset {
            scroll_area = scroll_area.vertical_scroll_offset(offset);
        }
        let mut context_command = None;
        let palette = app_palette(options.dark);
        let output = scroll_area.show(ui, |ui| {
            let viewport = ui.available_size();
            ui.set_min_size(Vec2::new(viewport.x, viewport.y.max(420.0)));
            ui.add_space(28.0);
            let available_width = ui.available_width();
            let page_width = (available_width - 48.0)
                .clamp(280.0, 920.0)
                .min(available_width);
            let side_margin = ((available_width - page_width) * 0.5).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(side_margin);
                document_page_frame(palette, options.dark).show(ui, |ui| {
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.set_width((page_width - 112.0).max(160.0));
                        let available =
                            Vec2::new(ui.available_width(), (viewport.y - 144.0).max(360.0));
                        let row_height = ui.text_style_height(&egui::TextStyle::Monospace).max(1.0);
                        let desired_rows = (available.y / row_height).max(20.0) as usize;
                        let editor_id = ui.make_persistent_id(("editor", document.id()));
                        self.editor_widget_id = Some(editor_id);
                        let requested_cursor = self.pending_editor_cursor;
                        if let Some(cursor_range) = self.pending_editor_cursor.take() {
                            let mut state =
                                TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
                            state.cursor.set_char_range(Some(cursor_range));
                            state.store(ui.ctx(), editor_id);
                            ui.memory_mut(|memory| memory.request_focus(editor_id));
                            self.editor_cursor = Some(cursor_range);
                        }
                        let input_action = editor_input_action(ui);
                        let mut editor_buffer = TrackingTextBuffer::new(&mut document.content);
                        let output = TextEdit::multiline(&mut editor_buffer)
                            .id(editor_id)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .hint_text("Markdown 源码编辑区")
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
                        if let Some(cursor_range) = cursor_after {
                            self.editor_cursor = Some(cursor_range);
                        }
                        // egui can surrender focus after accepting Tab. Finish
                        // the smart edit for input this widget already consumed.
                        let accepted_input =
                            output.response.has_focus() || output.response.changed();
                        let mut changed = output.response.changed();
                        let mut kind = EditKind::Typing;
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
        self.finish_history_action(document, effects);
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
