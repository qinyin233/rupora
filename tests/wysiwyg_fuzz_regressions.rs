use rupora::wysiwyg::VisualProjection;

#[test]
fn long_backslash_runs_keep_escape_pairs_editable() {
    for count in [1, 2, 3, 31, 32, 32_768] {
        let source = format!("前 {}* 后", "\\".repeat(count));
        let projection = VisualProjection::from_markdown(&source);
        assert_eq!(
            projection.text(),
            format!("前 {}* 后", "\\".repeat(count / 2))
        );
        let punctuation = 2 + count / 2;
        assert_eq!(
            projection.source_char_range(&source, punctuation..punctuation + 1),
            2 + count - count % 2..3 + count
        );
        let edited = format!("前 {} 后", "\\".repeat(count / 2));
        let update = projection
            .apply_edit(&source, &edited, punctuation..punctuation)
            .unwrap();
        assert_eq!(
            update.source,
            format!("前 {} 后", "\\".repeat(count - count % 2))
        );
        if count >= 2 {
            assert_eq!(projection.source_char_range(&source, 2..3), 2..4);
        }
    }
}

#[test]
fn replacing_escaped_punctuation_does_not_leave_an_escape_behind() {
    for punctuation in (b'!'..=b'~')
        .filter(u8::is_ascii_punctuation)
        .map(char::from)
    {
        let source = format!("前 \\{punctuation} 后");
        let projection = VisualProjection::from_markdown(&source);
        assert_eq!(projection.text(), format!("前 {punctuation} 后"));
        for replacement in ["", "新🙂"] {
            let edited = format!("前 {replacement} 后");
            let cursor = 2 + replacement.chars().count();
            let update = projection
                .apply_edit(&source, &edited, cursor..cursor)
                .unwrap();
            assert_eq!(update.source, edited, "source={source:?}");
            assert_eq!(
                VisualProjection::from_markdown(&update.source).text(),
                edited
            );
        }
    }
}

#[test]
fn multiline_hidden_link_destinations_do_not_insert_visual_blank_lines() {
    for (source, expected) in [
        ("[中文](\nnotes.md\n)**后文**", "中文后文"),
        ("[中文](notes.md\n\"多行标题\")*后文*", "中文后文"),
        ("![图](\nimage.png\n)**后文**", "▧ 图后文"),
    ] {
        let projection = VisualProjection::from_markdown(source);
        assert_eq!(projection.text(), expected, "source={source:?}");
        let edited = expected.replace("后文", "新🙂");
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, source.replace("后文", "新🙂"));
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited
        );
    }
}

#[test]
fn deleting_a_whole_image_removes_its_hidden_destination() {
    for image in [
        "![中文🙂](assets/image.png \"标题\")",
        "![**中文🙂**](assets/image.png)",
        "[![中文🙂](assets/image.png)](notes.md)",
    ] {
        let source = format!("前 {image} 后");
        let projection = VisualProjection::from_markdown(&source);
        assert_eq!(projection.text(), "前 ▧ 中文🙂 后");
        let update = projection.apply_edit(&source, "前  后", 2..2).unwrap();
        assert_eq!(update.source, "前  后");

        let update = projection
            .apply_edit(&source, "前 ▧ 中新🙂 后", 6..6)
            .unwrap();
        assert_eq!(update.source, source.replace("中文🙂", "中新🙂"));
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            "前 ▧ 中新🙂 后"
        );
    }
}

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
