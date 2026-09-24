use std::ops::Range;

use rupora::wysiwyg::VisualProjection;

const MAX_INPUT_BYTES: usize = 256 * 1024;
const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_REPLACEMENT_BYTES: usize = 4 * 1024;
const MAX_STEPS: usize = 32;

// Shared by libFuzzer and ordinary integration tests. The bytecode starts with
// a u32 LEB128 source length and UTF-8 source, followed by edits:
// flags:u8, first:LEB128, second:LEB128, replacement_length:LEB128, replacement.
// Lengths are reduced to their budgets; positions cover the full current visual
// text. Bit 0 uses the last source selection to reveal syntax; bit 1 selects the
// replacement instead of leaving a caret at its end. Invalid/incomplete fields
// end the trace. This exercises projection edits, not App/IME event routing.
pub fn run(data: &[u8]) -> Option<(String, Range<usize>, usize)> {
    if data.len() > MAX_INPUT_BYTES {
        return None;
    }
    let mut input = Input(data);
    let source_bytes = input.number()? % (MAX_SOURCE_BYTES + 1);
    let mut source = std::str::from_utf8(input.take(source_bytes)?)
        .ok()?
        .to_owned();
    let mut selection = 0..0;
    let mut steps = 0;
    check_projection(&source, &VisualProjection::from_markdown(&source));

    while steps < MAX_STEPS && source.len() <= MAX_SOURCE_BYTES {
        let Some(edit) = input.edit() else { break };
        let projection = VisualProjection::from_markdown_with_selection(
            &source,
            (edit.flags & 1 != 0).then(|| selection.clone()),
        );
        check_projection(&source, &projection);
        let length = projection.text().chars().count();
        let first = edit.first % (length + 1);
        let second = edit.second % (length + 1);
        let removed = first.min(second)..first.max(second);
        let replacement = String::from_utf8_lossy(edit.replacement);
        let mut edited = projection.text().to_owned();
        let start = char_to_byte(&edited, removed.start);
        let end = char_to_byte(&edited, removed.end);
        edited.replace_range(start..end, &replacement);
        let cursor = removed.start + replacement.chars().count();
        let visual_selection = if edit.flags & 2 != 0 {
            removed.start..cursor
        } else {
            cursor..cursor
        };
        check_runs(&projection, &edited);

        let update = projection.apply_edit(&source, &edited, visual_selection.clone());
        assert_eq!(update.is_none(), edited == projection.text());
        if let Some(update) = update {
            check_range(&update.selection, update.source.chars().count());
            source = update.source;
            selection = update.selection;
        } else {
            selection = projection.source_char_range(&source, visual_selection);
            check_range(&selection, source.chars().count());
        }
        steps += 1;
    }

    // A single edit may cross the size budget. Stop before parsing the enlarged
    // source again; its returned selection was still validated above.
    if source.len() <= MAX_SOURCE_BYTES {
        check_projection(
            &source,
            &VisualProjection::from_markdown_with_selection(&source, Some(selection.clone())),
        );
    }
    Some((source, selection, steps))
}

fn check_range(range: &Range<usize>, length: usize) {
    assert!(range.start <= range.end, "reversed range {range:?}");
    assert!(range.end <= length, "range {range:?} exceeds {length}");
}

fn check_runs(projection: &VisualProjection, text: &str) {
    let mut covered = 0;
    for run in projection.runs_for(text) {
        assert_eq!(
            run.range.start, covered,
            "style runs overlap or leave a gap"
        );
        assert!(run.range.start < run.range.end, "empty style run");
        covered = run.range.end;
    }
    assert_eq!(covered, text.chars().count(), "style runs miss visual text");
}

fn check_projection(source: &str, projection: &VisualProjection) {
    check_runs(projection, projection.text());
    let source_length = source.chars().count();
    let visual_length = projection.text().chars().count();
    let mut previous = 0;
    // Sampling stays linear in document length, unlike testing every range.
    for point in [
        0,
        visual_length / 4,
        visual_length / 2,
        visual_length * 3 / 4,
        visual_length,
    ] {
        let cursor = projection.source_char_range(source, point..point);
        check_range(&cursor, source_length);
        assert!(cursor.start >= previous, "source mapping moves backwards");
        previous = cursor.start;
        check_range(
            &projection.source_char_range(source, 0..point),
            source_length,
        );
        check_range(
            &projection.source_char_range(source, point..visual_length),
            source_length,
        );
    }
    for point in [0, source_length / 2, source_length] {
        check_range(
            &projection.visual_char_range(source, point..source_length),
            visual_length,
        );
    }
}

struct Input<'a>(&'a [u8]);

struct Edit<'a> {
    flags: u8,
    first: usize,
    second: usize,
    replacement: &'a [u8],
}

impl<'a> Input<'a> {
    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(length)?;
        self.0 = tail;
        Some(head)
    }

    fn number(&mut self) -> Option<usize> {
        let mut value = 0_u32;
        for shift in [0, 7, 14, 21, 28] {
            let byte = self.take(1)?[0];
            if shift == 28 && byte > 0x0f {
                return None;
            }
            value |= u32::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value as usize);
            }
        }
        None
    }

    fn edit(&mut self) -> Option<Edit<'a>> {
        let flags = self.take(1)?[0];
        let first = self.number()?;
        let second = self.number()?;
        let length = self.number()? % (MAX_REPLACEMENT_BYTES + 1);
        Some(Edit {
            flags,
            first,
            second,
            replacement: self.take(length)?,
        })
    }
}

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte, _)| byte)
}
