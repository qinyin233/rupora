use rupora::wysiwyg::VisualProjection;

#[test]
fn deleting_images_between_formatted_spans_keeps_text_and_caret() {
    for source in [
        "**前**![*中*](x)**后**",
        "*前*![中](x)*后*",
        "~~前~~![中](x)~~后~~",
        "***前***![中](x)***后***",
        "*外 **前**![中](x)**后** 外*",
        "[**前**![中](x)**后**](outer)",
        "![**前**![中](x)**后**](outer)",
        "**前**![中](x)*后*",
        "**前**![中](x)\n**后**",
    ] {
        let projection = VisualProjection::from_markdown(source);
        let image = projection.text().find("▧ 中").unwrap();
        let cursor = projection.text()[..image].chars().count();
        let mut edited = projection.text().to_owned();
        edited.replace_range(image..image + "▧ 中".len(), "");
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        let reparsed = VisualProjection::from_markdown(&update.source);
        assert_eq!(
            reparsed.text(),
            edited,
            "source={source:?}, update={update:?}"
        );
        assert_eq!(
            reparsed.visual_char_range(&update.source, update.selection),
            cursor..cursor,
            "source={source:?}",
        );
        let original_runs = projection.runs_for(projection.text());
        let updated_runs = reparsed.runs_for(reparsed.text());
        for character in ['前', '后'] {
            let original_index = projection
                .text()
                .chars()
                .position(|ch| ch == character)
                .unwrap();
            let updated_index = reparsed
                .text()
                .chars()
                .position(|ch| ch == character)
                .unwrap();
            assert_eq!(
                original_runs
                    .iter()
                    .find(|run| run.range.contains(&original_index))
                    .map(|run| run.style),
                updated_runs
                    .iter()
                    .find(|run| run.range.contains(&updated_index))
                    .map(|run| run.style),
                "source={source:?}, character={character}",
            );
        }
    }
}

#[test]
fn edits_across_image_alt_and_adjacent_formatting_keep_balanced_wrappers() {
    for (source, edited, cursor, expected) in [
        ("![**甲**](a)*乙*", "▧ 新", 3, "![新](a)"),
        ("[![**甲**](a)](outer)*乙*", "▧ 新", 3, "[![新](a)](outer)"),
        ("![a ![b](i) c](outer)", "▧ a ▧ b", 7, "![a ![b](i)](outer)"),
    ] {
        let projection = VisualProjection::from_markdown_with_selection(
            source,
            Some(source.chars().count()..source.chars().count()),
        );
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, expected, "source={source:?}");
        let reparsed = VisualProjection::from_markdown(&update.source);
        assert_eq!(reparsed.text(), edited);
        assert_eq!(
            reparsed.visual_char_range(&update.source, update.selection),
            cursor..cursor,
        );
    }
}

#[test]
fn removing_an_image_marker_keeps_alt_formatting_and_outer_links() {
    for (image, unwrapped) in [
        ("![中文🙂](image.png \"标题\")", "中文🙂"),
        ("![**中文🙂**](image.png)", "**中文🙂**"),
        (
            "[![**中文🙂**](image.png)](notes.md)",
            "[**中文🙂**](notes.md)",
        ),
    ] {
        let source = format!("前 {image} 后");
        let projection = VisualProjection::from_markdown(&source);
        for (edited, cursor) in [
            ("前 中文🙂 后", 2),
            ("前 ▧中文🙂 后", 3),
            ("前  中文🙂 后", 2),
        ] {
            let update = projection
                .apply_edit(&source, edited, cursor..cursor)
                .unwrap();
            assert_eq!(update.source, format!("前 {unwrapped} 后"));
            let reparsed = VisualProjection::from_markdown(&update.source);
            assert_eq!(reparsed.text(), "前 中文🙂 后");
            assert_eq!(
                reparsed.visual_char_range(&update.source, update.selection),
                2..2
            );
        }
    }

    for (source, edited, cursor, expected) in [
        (
            "前 ![**中文🙂**](image.png) 后",
            "前 文🙂 后",
            2,
            "前 **文🙂** 后",
        ),
        (
            "前 [![**中文🙂**](image.png)](notes.md) 后",
            "前 新文🙂 后",
            3,
            "前 [新**文🙂**](notes.md) 后",
        ),
        ("![甲](a.png)![**乙🙂**](b.png)", "乙🙂", 0, "**乙🙂**"),
    ] {
        let projection = VisualProjection::from_markdown(source);
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, expected);
        let reparsed = VisualProjection::from_markdown(&update.source);
        assert_eq!(reparsed.text(), edited);
        assert_eq!(
            reparsed.visual_char_range(&update.source, update.selection),
            cursor..cursor
        );
    }
}

#[test]
fn partial_entity_edits_keep_retained_punctuation_literal() {
    for (source, edited, cursor) in [
        ("&nvgt;", ">", 1),
        ("&nvlt;正文", "<!--正文", 4),
        ("**&nvlt;正文**", "<!--正文", 4),
    ] {
        let projection = VisualProjection::from_markdown(source);
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();
        let reparsed = VisualProjection::from_markdown(&update.source);
        assert_eq!(
            reparsed.text(),
            edited,
            "source={source:?}, update={update:?}"
        );
        assert_eq!(
            reparsed.visual_char_range(&update.source, update.selection),
            cursor..cursor
        );
    }
}

#[test]
fn replacing_complete_images_removes_the_image_syntax_and_keeps_outer_links() {
    for (image, expected) in [
        ("![中文🙂](assets/image.png \"标题\")", "替换🙂"),
        ("![**中文🙂**](assets/image.png)", "替换🙂"),
        (
            "[![中文🙂](assets/image.png)](notes.md)",
            "[替换🙂](notes.md)",
        ),
    ] {
        let source = format!("前 {image} 后");
        let projection = VisualProjection::from_markdown(&source);
        let edited = "前 替换🙂 后";
        let update = projection.apply_edit(&source, edited, 5..5).unwrap();
        assert_eq!(update.source, format!("前 {expected} 后"));
        let reparsed = VisualProjection::from_markdown(&update.source);
        assert_eq!(reparsed.text(), edited);
        assert_eq!(
            reparsed.visual_char_range(&update.source, update.selection),
            5..5
        );
    }
}

#[test]
fn decoded_entities_keep_their_full_source_mapping_during_replacement() {
    for entity in [
        "&amp;", "&#38;", "&semi;", "&#35;", "&fjlig;", "&lt;", "&quot;",
    ] {
        let source = format!("前 {entity} 后");
        let projection = VisualProjection::from_markdown(&source);
        for replacement in ["", "新🙂"] {
            let edited = format!("前 {replacement} 后");
            let cursor = 2 + replacement.chars().count();
            let update = projection
                .apply_edit(&source, &edited, cursor..cursor)
                .unwrap();
            assert_eq!(update.source, edited, "source={source:?}");
            assert_eq!(update.selection, cursor..cursor);
        }
    }
}

#[test]
fn partial_entity_edits_preserve_unselected_decoded_characters() {
    for (source, edited, cursor, expected) in [
        ("前 &fjlig; 后", "前 Xj 后", 3, "前 Xj 后"),
        ("前 &fjlig; 后", "前 f🙂 后", 4, "前 f🙂 后"),
        ("前 &fjlig; 后", "前 f新j 后", 4, "前 f新j 后"),
        (
            "前 &NotEqualTilde; 后",
            "前 新\u{338} 后",
            3,
            "前 新\u{338} 后",
        ),
        ("前 &NotEqualTilde; 后", "前 ≂ 后", 3, "前 ≂ 后"),
        ("\\*&fjlig;&amp;尾", "*f新&尾", 3, "\\*f新&amp;尾"),
        (
            "A&fjlig;&NotEqualTilde;Z",
            "Af新\u{338}Z",
            3,
            "Af新\u{338}Z",
        ),
        ("**&fjlig;**", "新j", 1, "**新j**"),
    ] {
        let projection = VisualProjection::from_markdown(source);
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, expected, "source={source:?}");
        let reparsed = VisualProjection::from_markdown(&update.source);
        assert_eq!(reparsed.text(), edited);
        assert_eq!(
            reparsed.visual_char_range(&update.source, update.selection),
            cursor..cursor
        );
    }

    let source = "前&fjlig;后";
    let projection = VisualProjection::from_markdown(source);
    let update = projection.apply_edit(source, "前f新🙂j后", 2..4).unwrap();
    assert_eq!(update.source, "前f新🙂j后");
    assert_eq!(update.selection, 2..4);
}

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
