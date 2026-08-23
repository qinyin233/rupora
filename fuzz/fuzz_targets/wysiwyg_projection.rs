#![no_main]

use libfuzzer_sys::fuzz_target;
use rupora::wysiwyg::VisualProjection;

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 || data.len() < 3 {
        return;
    }
    let split = 2 + usize::from(data[0]) % (data.len() - 1);
    let Ok(source) = std::str::from_utf8(&data[1..split]) else {
        return;
    };
    let projection = VisualProjection::from_markdown(source);
    let visual_len = projection.text().chars().count();
    let first = usize::from(data[1]) % (visual_len + 1);
    let second = usize::from(*data.get(split).unwrap_or(&0)) % (visual_len + 1);
    let selection = first.min(second)..first.max(second);
    let replacement = String::from_utf8_lossy(data.get(split + 1..).unwrap_or_default());
    let mut edited = projection.text().to_owned();
    let start = char_to_byte(&edited, selection.start);
    let end = char_to_byte(&edited, selection.end);
    edited.replace_range(start..end, &replacement);
    let cursor = selection.start + replacement.chars().count();

    if let Some(update) = projection.apply_edit(source, &edited, cursor..cursor) {
        assert!(update.selection.start <= update.selection.end);
        assert!(update.selection.end <= update.source.chars().count());
        let reparsed = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert!(reparsed.text().is_char_boundary(reparsed.text().len()));
    }
});

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte, _)| byte)
}
