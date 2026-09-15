//! Read-only document preview with explicit edit and navigation outcomes.
use super::*;

impl EditorSurface {
    pub(super) fn preview_pane(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        options: &EditorOptions<'_>,
        effects: &mut EditorOutput,
        scroll_offset: Option<f32>,
    ) -> PaneScroll {
        let viewport_height = ui.available_height();
        let document_id = document.id();
        let view = document.render_view();
        let source = view.source;
        let references = view.references;
        let blocks = view.blocks;
        let scroll_to_block = if options.mode == ViewMode::Preview {
            self.pending_editor_cursor
                .take()
                .map(|cursor| block_for_char_index(source, blocks, cursor.primary.index.0).id)
        } else {
            None
        };
        let base_path = options.base_path;
        let prepared_document =
            self.render_cache
                .prepare_document(document_id, source, blocks, references.clone());
        let dark = options.dark;
        let palette = app_palette(options.dark);
        let mut task_toggle = None;
        let mut clicked_destination = None;
        let mut scroll_area = ScrollArea::vertical().id_salt(("preview-scroll", document_id));
        if let Some(offset) = scroll_offset {
            scroll_area = scroll_area.vertical_scroll_offset(offset);
        }
        let output = scroll_area.show(ui, |ui| {
            let page_layout = document_page_layout(ui.available_width(), viewport_height);
            ui.add_space(page_layout.top_margin);
            ui.horizontal(|ui| {
                ui.add_space(page_layout.side_margin);
                document_page_frame(palette, options.dark, &page_layout).show(ui, |ui| {
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.set_width(page_layout.content_width);
                        ui.set_min_height(page_layout.min_content_height);
                        for block in blocks {
                            let source_block = &source[block.range.clone()];
                            let prepared = prepared_document.block(
                                ui.ctx(),
                                block,
                                ui.available_width(),
                                dark,
                                base_path,
                            );
                            let preview_source = prepared.preview_source();
                            let code = fenced_code_content(source_block);
                            let preview = prepared.show(ui);
                            let response = ui.interact(
                                preview.rect,
                                ui.make_persistent_id((
                                    "native-preview-block",
                                    document_id,
                                    block.id,
                                )),
                                egui::Sense::click(),
                            );
                            if scroll_to_block == Some(block.id) {
                                ui.scroll_to_rect(preview.rect, Some(Align::Min));
                            }
                            if response.clicked()
                                && let Some(position) = response.interact_pointer_pos()
                            {
                                let local_source_byte = preview.mapping.source_byte_at_position(
                                    preview_source,
                                    preview.rect,
                                    position,
                                );
                                if !prepared.is_generated()
                                    && let Some(updated) = markdown::toggle_task_marker_at(
                                        source_block,
                                        local_source_byte,
                                    )
                                {
                                    task_toggle = Some((block.range.clone(), updated));
                                } else if let Some(destination) =
                                    markdown::link_destination_with_references(
                                        preview_source,
                                        local_source_byte,
                                        &references,
                                    )
                                {
                                    clicked_destination = Some(destination);
                                }
                            }
                            if let Some(code) = code {
                                let _ = show_code_copy_button(
                                    ui,
                                    (document_id, block.id),
                                    preview.rect,
                                    code,
                                    palette,
                                );
                            }
                            ui.add_space(12.0);
                        }
                        ui.add_space(60.0);
                    });
                });
            });
            ui.add_space(40.0);
        });
        if let Some((range, updated)) = task_toggle {
            document.edit(EditKind::TaskList, None, |content| {
                content.replace_range(range, &updated);
                None
            });
            effects.notice = "已更新任务列表".to_owned();
        }
        if let Some(destination) = clicked_destination {
            effects.destination = Some(destination);
        }
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
