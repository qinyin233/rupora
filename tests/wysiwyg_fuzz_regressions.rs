use rupora::wysiwyg::VisualProjection;

#[test]
fn scheduled_fuzz_projection_crash_d39e1582() {
    // Exact input reported by Native Rust CI run 34099943888.
    let data = b".+\n  =+\n   $       b";
    let split = 2 + usize::from(data[0]) % (data.len() - 1);
    let source = std::str::from_utf8(&data[1..split]).unwrap();
    let projection = VisualProjection::from_markdown(source);
    let visual_len = projection.text().chars().count();
    let first = usize::from(data[1]) % (visual_len + 1);
    let second = usize::from(*data.get(split).unwrap_or(&0)) % (visual_len + 1);
    let selection = first.min(second)..first.max(second);
    let replacement = String::from_utf8_lossy(data.get(split + 1..).unwrap_or_default());
    let mut edited = projection.text().to_owned();
    let byte_at = |index| {
        edited
            .char_indices()
            .nth(index)
            .map_or(edited.len(), |(byte, _)| byte)
    };
    edited.replace_range(
        byte_at(selection.start)..byte_at(selection.end),
        &replacement,
    );
    let cursor = selection.start + replacement.chars().count();
    let update = projection
        .apply_edit(source, &edited, cursor..cursor)
        .unwrap();
    assert!(update.selection.start <= update.selection.end);
    assert!(update.selection.end <= update.source.chars().count());
    let reparsed =
        VisualProjection::from_markdown_with_selection(&update.source, Some(update.selection));
    assert!(reparsed.text().is_char_boundary(reparsed.text().len()));
}

#[test]
fn trailing_list_whitespace_is_rendered_once_and_remains_editable() {
    let source = "+ \n  ";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "• \n  ");
    let update = projection.apply_edit(source, "• \nx ", 4..4).unwrap();
    assert_eq!(update.source, "+ \nx ");
    assert_eq!(update.selection, 4..4);
}

#[test]
fn selections_across_trailing_block_whitespace_keep_source_order() {
    for block in [
        "+ ",
        "- 中文🙂",
        "> 引用",
        "# 标题",
        "plain",
        "```\ncode\n```",
    ] {
        for newline in ["\n", "\r\n", "\r"] {
            for whitespace in [" ", "  ", "\t", " \t "] {
                let source = format!("{block}{newline}{whitespace}");
                let projection = VisualProjection::from_markdown(&source);
                let length = projection.text().chars().count();
                for start in 0..=length {
                    for end in start..=length {
                        let range = projection.source_char_range(&source, start..end);
                        assert!(
                            range.start <= range.end,
                            "{source:?}: {start}..{end} => {range:?}; {projection:?}"
                        );
                        assert!(range.end <= source.chars().count());
                    }
                }
                let mut edited = projection.text().to_owned();
                edited.pop();
                let cursor = edited.chars().count();
                let update = projection
                    .apply_edit(&source, &edited, cursor..cursor)
                    .unwrap();
                let mut expected = source.clone();
                expected.pop();
                assert_eq!(update.source, expected, "{source:?}");
            }
        }
    }
}
