//! Input ordering and source-coordinate boundary operations.
use super::*;

impl EditorSurface {
    /// Apply only the leading run of unmodified Tab/Shift+Tab events. Each
    /// command uses the updated full source selection before widgets can replace
    /// it with a literal tab. Other input and IME events retain their order.
    pub(super) fn apply_leading_tab_input(&mut self, ui: &mut Ui, document: &mut Document) -> bool {
        if self.hybrid_ime_session.is_some() {
            return false;
        }
        let mut consumed = false;
        loop {
            let Some(index) = leading_editor_event(ui, self.editor_widget_id) else {
                break;
            };
            let Some(egui::Event::Key {
                key: Key::Tab,
                modifiers,
                ..
            }) = ui.input(|input| input.events.get(index).cloned())
            else {
                break;
            };
            if modifiers.ctrl || modifiers.alt || modifiers.mac_cmd {
                break;
            }
            let Some(cursor) = self.pending_editor_cursor.or(self.editor_cursor) else {
                break;
            };
            let selected = clamp_char_range(&document.content, cursor_range_to_char_range(cursor));
            let mut next = selected.clone();
            document.edit(EditKind::Other, Some(selected.clone()), |source| {
                next = editing::indent_selected_lines(source, selected, modifiers.shift);
                Some(next.clone())
            });
            self.hybrid_pointer_anchor = None;
            self.hybrid_cross_selection = None;
            self.select_cursor(cursor_range_with_direction(next, cursor));
            ui.input_mut(|input| {
                input.events.remove(index);
            });
            // Focus navigation was derived from the original raw event batch.
            // These keys have been handled as indentation commands instead.
            ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            consumed = true;
        }
        consumed
    }
}

pub(super) fn hybrid_page_cursor(
    ui: &Ui,
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    cursor: usize,
    down: bool,
    dark: bool,
) -> usize {
    let width = document_page_layout(ui.available_width(), ui.available_height()).content_width;
    let references = markdown::reference_definitions(source);
    let mut layouts = Vec::new();
    let mut top = 0.0;
    let active = block_for_char_index(source, blocks, cursor).id;
    let mut caret = egui::Pos2::ZERO;
    for block in blocks {
        let text = &source[block.range.clone()];
        let projection =
            VisualProjection::from_markdown_with_context(text, None, references.clone());
        let galley = wysiwyg_layout(
            ui,
            projection.text(),
            &projection,
            width,
            app_palette(dark),
            true,
        );
        if block.id == active {
            let start = source[..block.range.start].chars().count();
            let local = cursor.saturating_sub(start);
            let visual = projection.visual_char_range(text, local..local).start;
            caret = galley.pos_from_cursor(CCursor::new(visual)).min + egui::vec2(0.0, top);
        }
        let height = galley.size().y.max(1.0) + 20.0;
        layouts.push((block, projection, galley, top, height));
        top += height;
    }
    let target_y = (caret.y
        + if down {
            ui.available_height() * 0.85
        } else {
            -ui.available_height() * 0.85
        })
    .clamp(0.0, top);
    let (block, projection, galley, top, _) = layouts
        .iter()
        .find(|(_, _, _, top, height)| target_y < top + height)
        .unwrap_or_else(|| layouts.last().unwrap());
    let visual = galley
        .cursor_from_pos(egui::vec2(caret.x, (target_y - top).max(0.0)))
        .index
        .0;
    source[..block.range.start].chars().count()
        + projection
            .source_char_range(&source[block.range.clone()], visual..visual)
            .start
}

pub(super) fn hybrid_document_edge(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    end: bool,
) -> usize {
    let block = if end { blocks.last() } else { blocks.first() };
    if let Some(block) = block {
        let text = &source[block.range.clone()];
        if is_fenced_code_block(text) {
            let projection = VisualProjection::from_markdown(text);
            let at = if end {
                projection.text().chars().count()
            } else {
                0
            };
            return source[..block.range.start].chars().count()
                + projection.source_char_range(text, at..at).start;
        }
    }
    if end { source.chars().count() } else { 0 }
}

pub(super) fn block_for_char_index<'a>(
    source: &str,
    blocks: &'a [markdown::MarkdownBlock],
    char_index: usize,
) -> &'a markdown::MarkdownBlock {
    let byte_index = source
        .char_indices()
        .nth(char_index)
        .map_or(source.len(), |(index, _)| index);
    blocks
        .iter()
        .find(|block| {
            (block.range.start..block.range.end).contains(&byte_index)
                || (block.range.is_empty() && block.range.start == byte_index)
        })
        .or_else(|| {
            blocks.windows(2).find_map(|pair| {
                (pair[0].range.end <= byte_index
                    && byte_index < pair[1].range.start
                    && fenced_code_content(&source[pair[0].range.clone()]).is_some())
                .then_some(&pair[1])
            })
        })
        // Inter-block whitespace belongs to the preceding block's editable
        // range. Keeping the cursor there prevents a queued blank-line click
        // from being clamped to the beginning of the following paragraph on
        // the next frame. An exact next-block start was handled above.
        .or_else(|| {
            blocks
                .iter()
                .rev()
                .find(|block| block.range.start < byte_index)
        })
        .or_else(|| blocks.iter().find(|block| block.range.start >= byte_index))
        .unwrap_or_else(|| blocks.last().expect("Markdown always has an editing block"))
}

pub(crate) fn cursor_range_to_char_range(range: CCursorRange) -> std::ops::Range<usize> {
    let [start, end] = range.sorted_cursors();
    start.index.0..end.index.0
}

pub(super) fn cursor_range_saturating_sub(range: CCursorRange, offset: usize) -> CCursorRange {
    CCursorRange {
        primary: CCursor {
            index: range.primary.index.saturating_sub(offset),
            prefer_next_row: range.primary.prefer_next_row,
        },
        secondary: CCursor {
            index: range.secondary.index.saturating_sub(offset),
            prefer_next_row: range.secondary.prefer_next_row,
        },
        h_pos: range.h_pos,
    }
}

pub(super) fn cursor_range_add(range: CCursorRange, offset: usize) -> CCursorRange {
    CCursorRange {
        primary: range.primary + offset,
        secondary: range.secondary + offset,
        h_pos: range.h_pos,
    }
}

pub(super) fn cursor_range_with_direction(
    sorted: std::ops::Range<usize>,
    direction: CCursorRange,
) -> CCursorRange {
    if direction.primary.index >= direction.secondary.index {
        CCursorRange::two(CCursor::new(sorted.start), CCursor::new(sorted.end))
    } else {
        CCursorRange::two(CCursor::new(sorted.end), CCursor::new(sorted.start))
    }
}

pub(super) fn text_edit_cursor_after_input(
    output: &egui::text_edit::TextEditOutput,
) -> Option<CCursorRange> {
    if output.response.clicked() || output.response.dragged() {
        // Pointer interaction happens after `cursor_range` is captured. For
        // clicks and drag-selection the stored state is therefore newer, even
        // if another event also changed text in the same frame.
        output
            .state
            .cursor
            .range(&output.galley)
            .or(output.cursor_range)
    } else if output.response.changed() {
        // With hint text, egui 0.35 deliberately returns the pre-input empty
        // galley for the first keystroke. Resolving `state.cursor` against that
        // stale galley collapses the freshly advanced cursor back to zero, so
        // the next frame inserts before the Markdown marker. The public range
        // is the authoritative post-keyboard-edit cursor in this case.
        output
            .cursor_range
            .or_else(|| output.state.cursor.range(&output.galley))
    } else {
        output
            .state
            .cursor
            .range(&output.galley)
            .or(output.cursor_range)
    }
}

pub(super) fn text_edit_cursor_at_position(
    output: &egui::text_edit::TextEditOutput,
    position: egui::Pos2,
) -> CCursor {
    output
        .galley
        .cursor_from_pos(position - output.galley_pos + egui::vec2(output.galley.rect.left(), 0.0))
}

pub(super) fn source_selection_after_visual_input(
    projection: &VisualProjection,
    source: &str,
    previous_source_selection: Option<&std::ops::Range<usize>>,
    previous_visual_selection: Option<&std::ops::Range<usize>>,
    visual_selection: std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    if previous_visual_selection == Some(&visual_selection)
        && let Some(previous) = previous_source_selection
        && previous.end <= source.chars().count()
        && projection.visual_char_range(source, previous.clone()) == visual_selection
    {
        return previous.clone();
    }
    projection.source_char_range(source, visual_selection)
}

pub(super) fn line_break_before(source: &str, byte_index: usize) -> Option<std::ops::Range<usize>> {
    let before = source.get(..byte_index)?;
    if before.ends_with("\r\n") {
        Some(byte_index - 2..byte_index)
    } else if before.ends_with(['\n', '\r']) {
        Some(byte_index - 1..byte_index)
    } else {
        None
    }
}

pub(super) fn boundary_backspace_edit(
    source: &str,
    edit_range: std::ops::Range<usize>,
) -> Option<(std::ops::Range<usize>, usize)> {
    let line_break = line_break_before(source, edit_range.start)?;
    let cursor = source[..line_break.start].chars().count();
    Some((line_break.start..edit_range.end, cursor))
}

pub(super) fn hybrid_boundary_arrow_cursor(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    index: usize,
    cursor: usize,
    key: Key,
    references: Arc<markdown::ReferenceDefinitions>,
) -> Option<usize> {
    let forward = match key {
        Key::ArrowRight | Key::ArrowDown => true,
        Key::ArrowLeft | Key::ArrowUp => false,
        _ => return None,
    };
    let edge = |index: usize, at_end: bool| {
        let block = &blocks[index];
        let range = hybrid_edit_range(source, blocks, block.id);
        let text = &source[range.clone()];
        let code = is_fenced_code_block(text);
        let body_end = text.chars().count();
        let projection = VisualProjection::from_markdown_with_context(
            text,
            (at_end && !code).then_some(body_end..body_end),
            references.clone(),
        );
        let position = if at_end {
            projection.text().chars().count()
        } else {
            0
        };
        source[..range.start].chars().count()
            + projection.source_char_range(text, position..position).start
    };
    if cursor != edge(index, forward) {
        return None;
    }
    let block = &blocks[index];
    let start = source[..block.range.start].chars().count();
    let local = cursor.saturating_sub(start);
    if move_across_hidden_inline_code_boundary(
        &source[block.range.clone()],
        local..local,
        key == Key::ArrowLeft,
        key == Key::ArrowRight,
    )
    .is_some()
    {
        return None;
    }
    let adjacent = if forward {
        index + 1
    } else {
        index.checked_sub(1)?
    };
    blocks.get(adjacent)?;
    Some(edge(adjacent, !forward))
}

pub(super) fn paragraph_boundary_delete_range(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    index: usize,
    cursor: usize,
    key: Key,
) -> Option<std::ops::Range<usize>> {
    if !matches!(key, Key::Backspace | Key::Delete) {
        return None;
    }
    let block = &blocks[index];
    if is_fenced_code_block(&source[block.range.clone()]) {
        return None;
    }
    let byte = char_to_byte(source, cursor);
    let body = &source[block.range.clone()];
    let projection = VisualProjection::from_markdown(body);
    let local = source[block.range.start..byte.max(block.range.start).min(block.range.end)]
        .chars()
        .count();
    let visual = projection.visual_char_range(body, local..local).start;
    let byte = if key == Key::Backspace && visual == 0 {
        block.range.start
    } else if key == Key::Delete && visual == projection.text().chars().count() {
        block.range.end
    } else {
        byte
    };
    let mut range = if key == Key::Delete && byte == block.range.end {
        let next = blocks.get(index + 1)?;
        let end = if is_fenced_code_block(&source[next.range.clone()]) {
            line_break_before(source, next.range.start)?.start
        } else {
            next.range.start
        };
        byte..end
    } else if key == Key::Backspace && byte == block.range.start {
        let previous = blocks.get(index.checked_sub(1)?)?;
        let start = if is_fenced_code_block(&source[previous.range.clone()]) {
            nth_line_break_end(source, previous.range.end..byte, 0)?
        } else {
            previous.range.end
        };
        start..byte
    } else {
        return None;
    };
    let empty = if block.range.is_empty() {
        Some(index)
    } else if key == Key::Delete
        && blocks
            .get(index + 1)
            .is_some_and(|next| next.range.is_empty())
    {
        Some(index + 1)
    } else if key == Key::Backspace && index > 0 && blocks[index - 1].range.is_empty() {
        Some(index - 1)
    } else {
        None
    };
    if let Some(empty) = empty
        && empty > 0
        && let Some(next) = blocks.get(empty + 1)
    {
        let total = line_break_count(source, blocks[empty - 1].range.end..next.range.start);
        while !range.is_empty() && total.saturating_sub(line_break_count(source, range.clone())) < 2
        {
            range.end = line_break_before(source, range.end)?.start;
        }
    }
    (!range.is_empty()
        && source
            .get(range.clone())
            .is_some_and(|gap| gap.chars().all(char::is_whitespace)))
    .then(|| crate::wysiwyg::join_inline_paragraphs_range(source, range))
}

pub(super) fn fenced_boundary_delete_cursor(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    block_index: usize,
    selection: std::ops::Range<usize>,
    backspace: bool,
    delete: bool,
) -> Option<usize> {
    if !selection.is_empty() || (!backspace && !delete) {
        return None;
    }
    let block = blocks.get(block_index)?;
    let edit_range = hybrid_edit_range(source, blocks, block.id);
    let cursor_byte = edit_range.start + char_to_byte(&source[edit_range.clone()], selection.start);
    let edge_cursor = |target: &markdown::MarkdownBlock, at_end: bool| {
        let text = &source[target.range.clone()];
        let projection = VisualProjection::from_markdown(text);
        let edge = if at_end {
            projection.text().chars().count()
        } else {
            0
        };
        source[..target.range.start].chars().count()
            + projection.source_char_range(text, edge..edge).start
    };
    if delete {
        let next = blocks.get(block_index + 1)?;
        if is_fenced_code_block(&source[next.range.clone()])
            && source.get(cursor_byte..next.range.start) == Some("\n")
        {
            return Some(edge_cursor(next, false));
        }
    }
    if backspace {
        let previous = blocks.get(block_index.checked_sub(1)?)?;
        let previous_text = &source[previous.range.clone()];
        let previous_end =
            previous.range.start + previous_text.trim_end_matches(['\r', '\n']).len();
        if cursor_byte == block.range.start
            && fenced_code_content(previous_text).is_some()
            && source.get(previous_end..cursor_byte) == Some("\n")
        {
            return Some(edge_cursor(previous, true));
        }
        let block_text = &source[block.range.clone()];
        if fenced_code_content(block_text).is_some_and(|body| !body.is_empty())
            && cursor_byte == char_to_byte(source, edge_cursor(block, false))
        {
            return Some(edge_cursor(previous, true));
        }
    }
    None
}

pub(super) fn code_block_removal_range(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    block_index: usize,
) -> std::ops::Range<usize> {
    let block = &blocks[block_index];
    if let Some(next) = blocks.get(block_index + 1) {
        block.range.start..next.range.start
    } else if let Some(previous) = block_index
        .checked_sub(1)
        .and_then(|index| blocks.get(index))
    {
        previous.range.end..block.range.end
    } else {
        block.range.start..block.range.end.min(source.len())
    }
}

pub(super) fn paragraph_after_code_double_click(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    block_index: usize,
) -> (std::ops::Range<usize>, Option<String>, usize) {
    let block = &blocks[block_index];
    if let Some(next) = blocks.get(block_index + 1) {
        let cursor = source[..next.range.start].chars().count();
        return (next.range.start..next.range.start, None, cursor);
    }

    let tail_range = block.range.start..source.len();
    let mut replacement = source[tail_range.clone()].to_owned();
    let local_selection = paragraph_after_fenced_code(&mut replacement)
        .expect("a complete fenced block must expose a trailing paragraph");
    let cursor = source[..tail_range.start].chars().count() + local_selection.end;
    let changed = (replacement != source[tail_range.clone()]).then_some(replacement);
    (tail_range, changed, cursor)
}

pub(super) fn scroll_ratio(scroll: PaneScroll) -> f32 {
    if scroll.maximum <= f32::EPSILON {
        0.0
    } else {
        (scroll.offset / scroll.maximum).clamp(0.0, 1.0)
    }
}

pub(super) fn hybrid_edit_range(
    _source: &str,
    blocks: &[markdown::MarkdownBlock],
    active_id: BlockId,
) -> std::ops::Range<usize> {
    blocks
        .iter()
        .find(|block| block.id == active_id)
        .expect("active Markdown block must still exist")
        .range
        .clone()
}

pub(super) fn empty_paragraph_separators(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    index: usize,
) -> (usize, usize) {
    let block = &blocks[index];
    let before = index.checked_sub(1).map_or(0, |previous| {
        2usize.saturating_sub(line_break_count(
            source,
            blocks[previous].range.end..block.range.start,
        ))
    });
    let after = blocks.get(index + 1).map_or(0, |next| {
        2usize.saturating_sub(line_break_count(source, block.range.end..next.range.start))
    });
    (before, after)
}

pub(super) fn snap_atomic_cross_block_selection(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    anchor_block: BlockId,
    anchor_char: usize,
    current_block: BlockId,
    current_char: usize,
) -> CCursorRange {
    let snap = |block_id: BlockId, cursor: usize, lower_endpoint: bool| {
        let Some(block) = blocks.iter().find(|block| block.id == block_id) else {
            return cursor;
        };
        let Some(block_source) = source.get(block.range.clone()) else {
            return cursor;
        };
        if !is_fenced_code_block(block_source) {
            return cursor;
        }
        let boundary = if lower_endpoint {
            block.range.start
        } else {
            block.range.end
        };
        source[..boundary].chars().count()
    };

    let forward = anchor_char <= current_char;
    let snapped_anchor = snap(anchor_block, anchor_char, forward);
    let snapped_current = snap(current_block, current_char, !forward);
    CCursorRange {
        primary: CCursor::new(snapped_current),
        secondary: CCursor::new(snapped_anchor),
        h_pos: None,
    }
}

pub(super) fn line_break_count(source: &str, range: std::ops::Range<usize>) -> usize {
    let Some(fragment) = source.get(range) else {
        return 0;
    };
    let bytes = fragment.as_bytes();
    let mut cursor = 0usize;
    let mut count = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\r' => {
                cursor += usize::from(bytes.get(cursor + 1) == Some(&b'\n')) + 1;
                count += 1;
            }
            b'\n' => {
                cursor += 1;
                count += 1;
            }
            _ => cursor += 1,
        }
    }
    count
}

pub(super) fn nth_line_break_end(
    source: &str,
    range: std::ops::Range<usize>,
    target: usize,
) -> Option<usize> {
    let fragment = source.get(range.clone())?;
    let bytes = fragment.as_bytes();
    let mut cursor = 0usize;
    let mut index = 0usize;
    while cursor < bytes.len() {
        let length = match bytes[cursor] {
            b'\r' if bytes.get(cursor + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => {
                cursor += 1;
                continue;
            }
        };
        cursor += length;
        if index == target {
            return Some(range.start + cursor);
        }
        index += 1;
    }
    None
}

#[derive(Default)]
pub(super) struct EditorInputAction {
    pub(super) backspace: bool,
    pub(super) delete: bool,
    pub(super) down: bool,
    pub(super) enter: bool,
    pub(super) horizontal_modified: bool,
    pub(super) left: bool,
    pub(super) tab: bool,
    pub(super) right: bool,
    pub(super) select_all: bool,
    pub(super) shift: bool,
    pub(super) pasted_text: Option<String>,
    pub(super) typed_text: Option<String>,
}

pub(super) enum CrossBlockInput {
    Copy,
    Cut,
    Delete,
    Replace(String),
}

impl CrossBlockInput {
    pub(super) fn copy(&self) -> bool {
        matches!(self, Self::Copy | Self::Cut)
    }

    pub(super) fn replacement(&self) -> Option<&str> {
        match self {
            Self::Copy => None,
            Self::Cut | Self::Delete => Some(""),
            Self::Replace(text) => Some(text),
        }
    }
}

pub(super) fn take_cross_block_input(ui: &mut Ui) -> Option<CrossBlockInput> {
    let mut action = None;
    ui.input_mut(|input| {
        input.events.retain(|event| {
            // Apply one action to the cross-block range. Remaining events must
            // reach TextEdit at the resulting caret during this same frame.
            if action.is_some() {
                return true;
            }
            let next = match event {
                egui::Event::Copy => Some(CrossBlockInput::Copy),
                egui::Event::Cut => Some(CrossBlockInput::Cut),
                egui::Event::Paste(text) | egui::Event::Text(text) => Some(
                    CrossBlockInput::Replace(crate::document::normalize_line_endings(text)),
                ),
                egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty() => {
                    Some(CrossBlockInput::Replace(text.clone()))
                }
                egui::Event::Key {
                    key: Key::Backspace | Key::Delete,
                    pressed: true,
                    ..
                } => Some(CrossBlockInput::Delete),
                egui::Event::Key {
                    key: Key::Enter,
                    pressed: true,
                    modifiers,
                    ..
                } => Some(CrossBlockInput::Replace(
                    if modifiers.shift { "  \n" } else { "\n\n" }.to_owned(),
                )),
                _ => None,
            };
            if let Some(next) = next {
                action = Some(next);
                false
            } else {
                true
            }
        });
    });
    action
}

pub(super) fn normalize_editor_input_line_endings(ui: &mut Ui) {
    ui.input_mut(|input| {
        for event in &mut input.events {
            let text = match event {
                egui::Event::Text(text)
                | egui::Event::Paste(text)
                | egui::Event::Ime(egui::ImeEvent::Commit(text))
                | egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => text,
                _ => continue,
            };
            if text.contains('\r') {
                *text = crate::document::normalize_line_endings(text);
            }
        }
    });
}

/// Smart URL pastes apply to prose; code always receives the literal clipboard.
pub(super) fn selection_intersects_code(source: &str, selection: &std::ops::Range<usize>) -> bool {
    let start = char_to_byte(source, selection.start);
    let end = char_to_byte(source, selection.end);
    pulldown_cmark::Parser::new_ext(source, markdown::parser_options())
        .into_offset_iter()
        .any(|(event, range)| {
            matches!(
                event,
                pulldown_cmark::Event::Code(_)
                    | pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(_))
            ) && range.start < end
                && start < range.end
        })
}

pub(super) fn leading_editor_event(ui: &Ui, editor_id: Option<egui::Id>) -> Option<usize> {
    if editor_id.is_none() || ui.memory(|memory| memory.focused()) != editor_id {
        return None;
    }
    ui.input(|input| {
        input
            .events
            .iter()
            .enumerate()
            .find(|(_, event)| {
                matches!(
                    event,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Cut
                        | egui::Event::Copy
                        | egui::Event::Ime(_)
                        | egui::Event::PointerButton { pressed: true, .. }
                        | egui::Event::Key { pressed: true, .. }
                )
            })
            .map(|(index, _)| index)
    })
}

pub(super) fn leading_select_all_event(ui: &Ui, editor_id: Option<egui::Id>) -> Option<usize> {
    let index = leading_editor_event(ui, editor_id)?;
    ui.input(|input| {
        matches!(&input.events[index],
            egui::Event::Key { key: Key::A, pressed: true, modifiers, .. }
                if modifiers.command)
        .then_some(index)
    })
}

pub(super) fn has_leading_select_all(ui: &Ui, editor_id: Option<egui::Id>) -> bool {
    leading_select_all_event(ui, editor_id).is_some()
}

pub(super) fn take_leading_select_all(ui: &mut Ui, editor_id: Option<egui::Id>) -> bool {
    if let Some(index) = leading_select_all_event(ui, editor_id) {
        ui.input_mut(|input| {
            input.events.remove(index);
        });
        true
    } else {
        false
    }
}

pub(super) fn editor_input_action(ui: &Ui) -> EditorInputAction {
    ui.input(|input| {
        // TextEdit has already applied the whole event batch before our smart
        // editing hooks run. Replaying just its last event from the old buffer
        // would discard earlier text, deletions or cursor movements.
        let mut actions = input.events.iter().filter(|event| {
            matches!(
                event,
                egui::Event::Text(_)
                    | egui::Event::Paste(_)
                    | egui::Event::Cut
                    | egui::Event::Ime(_)
                    | egui::Event::PointerButton { pressed: true, .. }
                    | egui::Event::Key {
                        pressed: true,
                        key: Key::Backspace
                            | Key::Delete
                            | Key::Enter
                            | Key::Tab
                            | Key::Home
                            | Key::End
                            | Key::ArrowUp
                            | Key::ArrowDown
                            | Key::ArrowLeft
                            | Key::ArrowRight
                            | Key::PageUp
                            | Key::PageDown,
                        ..
                    }
            ) || matches!(event, egui::Event::Key {
                    key: Key::A, pressed: true, modifiers, ..
                } if modifiers.command)
        });
        let first_action = actions.next();
        let single_action = first_action.is_some() && actions.next().is_none();
        let modifiers = match (single_action, first_action) {
            (true, Some(egui::Event::Key { modifiers, .. })) => *modifiers,
            _ => input.modifiers,
        };
        EditorInputAction {
            backspace: single_action && input.key_pressed(Key::Backspace),
            delete: single_action && input.key_pressed(Key::Delete),
            down: single_action && input.key_pressed(Key::ArrowDown),
            enter: single_action && input.key_pressed(Key::Enter),
            horizontal_modified: modifiers.alt
                || modifiers.ctrl
                || modifiers.mac_cmd
                || modifiers.shift,
            left: single_action && input.key_pressed(Key::ArrowLeft),
            tab: single_action && input.key_pressed(Key::Tab),
            right: single_action && input.key_pressed(Key::ArrowRight),
            select_all: input.events.iter().any(|event| {
                matches!(event,
                egui::Event::Key { key: Key::A, pressed: true, modifiers, .. }
                    if modifiers.command)
            }),
            shift: modifiers.shift,
            pasted_text: input.events.iter().rev().find_map(|event| match event {
                egui::Event::Paste(text) if single_action => Some(text.trim().to_owned()),
                _ => None,
            }),
            typed_text: input.events.iter().rev().find_map(|event| match event {
                egui::Event::Text(text) if single_action => Some(text.clone()),
                _ => None,
            }),
        }
    })
}

pub(super) fn ime_frame_action(events: &[egui::Event]) -> ImeFrameAction {
    events
        .iter()
        .fold(ImeFrameAction::None, |action, event| match event {
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                if text.is_empty() {
                    ImeFrameAction::Cancel
                } else {
                    ImeFrameAction::Preedit
                }
            }
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                if text.is_empty() {
                    ImeFrameAction::Cancel
                } else {
                    ImeFrameAction::Commit
                }
            }
            _ => action,
        })
}

pub(super) fn clamp_char_range(
    text: &str,
    range: std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    let length = text.chars().count();
    range.start.min(length)..range.end.min(length).max(range.start.min(length))
}
