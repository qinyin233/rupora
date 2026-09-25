use rupora::wysiwyg::VisualProjection;

#[test]
fn incomplete_long_table_row_keeps_source_ranges_ordered() {
    let source = "| a | b |\n| - | - |\n| uuuuuuu";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a  │  b\n\nuuuuuuu  │  ");
    let length = projection.text().chars().count();
    for point in 0..=length {
        for range in [0..point, point..length] {
            let mapped = projection.source_char_range(source, range.clone());
            assert!(
                mapped.start <= mapped.end,
                "visual={:?} range={range:?} mapped={mapped:?}",
                projection.text()
            );
        }
    }
}

#[test]
fn oversized_inline_code_parser_range_does_not_delete_following_unicode() {
    let source = "# `a 00a¡  A¡`\\\r ¡A¡¡";
    let projection = VisualProjection::from_markdown(source);
    assert!(projection.text().contains("a 00a¡  A¡"));
    let edited = projection.text().replacen("a 00a¡  A¡", "", 1);
    let update = projection.apply_edit(source, &edited, 0..0).unwrap();
    assert!(update.source.ends_with("\\\r ¡A¡¡"), "{:?}", update.source);

    let after_closer_byte = source.find("`\\").unwrap() + 1;
    let after_closer = source[..after_closer_byte].chars().count();
    let body_end = source[..after_closer_byte - 1].chars().count();
    assert_eq!(
        rupora::wysiwyg::move_across_hidden_inline_code_boundary(
            source,
            after_closer..after_closer,
            true,
            false,
        ),
        Some(body_end..body_end)
    );
}

#[test]
fn tab_after_quote_marker_keeps_quote_and_nested_list_visible() {
    for (source, expected) in [
        (">\ta  b", "│ a  b"),
        ("  >\ta  b", "  │ a  b"),
        (">\t- item", "│ • item"),
        (">\t1. one", "│ 1. one"),
    ] {
        let projection = VisualProjection::from_markdown(source);
        assert_eq!(projection.text(), expected, "source={source:?}");
        let edited = format!("{expected}X");
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited,
            "source={source:?}, output={:?}",
            update.source
        );
    }
}

#[test]
fn quoted_indented_code_does_not_repeat_consumed_tabs_and_spaces() {
    for source in [">\t\ta", "> \t\ta", ">\t    a"] {
        let projection = VisualProjection::from_markdown(source);
        assert_eq!(projection.text(), "│   a", "source={source:?}");
        assert!(
            projection
                .runs_for(projection.text())
                .iter()
                .any(|run| run.style.code)
        );
    }
}

#[test]
fn quoted_fenced_code_keeps_its_content_indentation() {
    for source in [
        "> ```\n>   a\n> ```",
        "> ```\n>\ta\n> ```",
        ">\t```\n>\t  a\n>\t```",
    ] {
        assert_eq!(
            VisualProjection::from_markdown(source).text(),
            "│   a\n",
            "source={source:?}"
        );
    }
}

#[test]
fn replacing_first_formatted_character_with_space_does_not_expose_markers() {
    for (source, edited) in [
        ("*italic abc*", " talic abc"),
        ("**strong abc**", " trong abc"),
        ("~~struck text~~", " truck text"),
        ("**outer *inner* rest**", " uter inner rest"),
    ] {
        let projection = VisualProjection::from_markdown(source);
        let update = projection.apply_edit(source, edited, 1..1).unwrap();
        let reparsed = VisualProjection::from_markdown(&update.source);
        assert_eq!(
            reparsed.text(),
            edited,
            "updated source: {:?}",
            update.source
        );
        assert!(
            reparsed
                .runs_for(reparsed.text())
                .iter()
                .any(|run| run.style.emphasis || run.style.strong || run.style.strikethrough),
            "formatting was lost: {:?}",
            update.source
        );
        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert_eq!(active.text(), edited);
        assert_eq!(
            active.visual_char_range(&update.source, update.selection),
            1..1
        );
    }
}

#[test]
fn replacing_an_entire_emphasis_span_with_space_removes_empty_markers() {
    for (source, edited, cursor) in [
        ("a *em* b", "a   b", 3),
        ("*one* **two** *three*", "  two three", 1),
        ("**outer *inner* rest**", "outer   rest", 7),
    ] {
        let projection = VisualProjection::from_markdown(source);
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();
        if source == "a *em* b" {
            assert_eq!(update.source, "a   b");
        }
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited,
            "updated source: {:?}",
            update.source
        );
        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert_eq!(active.text(), edited);
        assert_eq!(
            active.visual_char_range(&update.source, update.selection),
            cursor..cursor
        );
    }
}

#[test]
fn editing_spaces_next_to_underscore_emphasis_keeps_markers_hidden() {
    let source = "prefix **strong** _em_ suffix";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "prefix strong em suffix");
    for edited in [
        "prefix strong新em suffix",
        "prefix strong新m suffix",
        "prefix strong em新suffix",
    ] {
        let cursor = edited.find('新').unwrap() + '新'.len_utf8();
        let cursor = edited[..cursor].chars().count();
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();
        let reparsed = VisualProjection::from_markdown(&update.source);
        assert_eq!(
            reparsed.text(),
            edited,
            "updated source: {:?}",
            update.source
        );
        assert!(
            reparsed
                .runs_for(reparsed.text())
                .iter()
                .any(|run| run.style.emphasis)
        );
        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert_eq!(active.text(), edited);
        assert_eq!(
            active.visual_char_range(&update.source, update.selection),
            cursor..cursor
        );
    }
}

#[test]
fn replacing_across_nested_emphasis_closer_keeps_outer_text_visible() {
    let source = "**outer *inner* rest**";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "outer inner rest");
    let edited = "outer inn est";
    let update = projection.apply_edit(source, edited, 10..10).unwrap();
    assert_eq!(
        VisualProjection::from_markdown(&update.source).text(),
        edited,
        "updated source: {:?}",
        update.source
    );
    let reparsed = VisualProjection::from_markdown(&update.source);
    let runs = reparsed.runs_for(reparsed.text());
    assert!(runs.iter().any(|run| run.style.strong));
    assert!(runs.iter().any(|run| run.style.emphasis));
    let active = VisualProjection::from_markdown_with_selection(
        &update.source,
        Some(update.selection.clone()),
    );
    assert_eq!(active.text(), edited);
    assert_eq!(
        active.visual_char_range(&update.source, update.selection),
        10..10
    );
    let next = "outer inn Xest";
    let follow_up = active.apply_edit(&update.source, next, 11..11).unwrap();
    assert_eq!(
        VisualProjection::from_markdown(&follow_up.source).text(),
        next
    );
}

#[test]
fn replacing_across_adjacent_emphasis_and_strong_keeps_both_styles() {
    let source = "*one* **two** *three*";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "one two three");
    let edited = "o wo three";
    let update = projection.apply_edit(source, edited, 2..2).unwrap();
    let reparsed = VisualProjection::from_markdown(&update.source);
    assert_eq!(
        reparsed.text(),
        edited,
        "updated source: {:?}",
        update.source
    );
    let runs = reparsed.runs_for(reparsed.text());
    assert!(runs.iter().any(|run| run.style.strong));
    assert!(runs.iter().any(|run| run.style.emphasis));
    let active = VisualProjection::from_markdown_with_selection(
        &update.source,
        Some(update.selection.clone()),
    );
    assert_eq!(active.text(), edited);
    assert_eq!(
        active.visual_char_range(&update.source, update.selection),
        2..2
    );
}

#[test]
fn replacing_text_across_emphasis_closer_keeps_plain_suffix_hidden() {
    let source = "a *em* b";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a em b");

    // Replace the final formatted character and the following plain space.
    let edited = "a e中🙂b";
    let update = projection.apply_edit(source, edited, 5..5).unwrap();
    let reparsed = VisualProjection::from_markdown(&update.source);
    assert_eq!(
        reparsed.text(),
        edited,
        "updated source: {:?}",
        update.source
    );
    assert_eq!(
        VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone())
        )
        .text(),
        edited
    );
    let runs = reparsed.runs_for(reparsed.text());
    assert!(runs.iter().any(|run| run.style.emphasis));
    assert!(
        runs.iter()
            .any(|run| !run.style.emphasis && run.range.contains(&5))
    );
}

#[test]
fn deleting_formatted_tail_keeps_the_remaining_space_visible_without_markers() {
    for (source, edited) in [
        ("*italic abc*", "italic "),
        ("**strong abc**", "strong "),
        ("**outer *inner* rest**", "outer inner "),
    ] {
        let projection = VisualProjection::from_markdown(source);
        let caret = edited.chars().count();
        let update = projection.apply_edit(source, edited, caret..caret).unwrap();
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited,
            "updated source: {:?}",
            update.source
        );
        let styles = VisualProjection::from_markdown(&update.source);
        let runs = styles.runs_for(styles.text());
        assert!(
            runs.iter()
                .any(|run| run.style.emphasis || run.style.strong)
        );
        assert_eq!(
            VisualProjection::from_markdown_with_selection(
                &update.source,
                Some(update.selection.clone())
            )
            .text(),
            edited
        );
        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        let next = format!("{edited}X");
        let caret = next.chars().count();
        let next_update = active
            .apply_edit(&update.source, &next, caret..caret)
            .unwrap();
        assert_eq!(
            VisualProjection::from_markdown(&next_update.source).text(),
            next
        );
    }
}

#[test]
fn replacing_across_adjacent_formatting_keeps_both_markers_hidden() {
    let source = "前**粗体**后*斜体*尾";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "前粗体后斜体尾");
    let edited = "前粗中🙂体尾";
    let update = projection.apply_edit(source, edited, 4..4).unwrap();
    assert_eq!(
        VisualProjection::from_markdown(&update.source).text(),
        edited,
        "updated source: {:?}",
        update.source
    );
    let styles = VisualProjection::from_markdown(&update.source);
    let runs = styles.runs_for(styles.text());
    assert!(runs.iter().any(|run| run.style.strong));
    assert!(runs.iter().any(|run| run.style.emphasis));
    assert_eq!(
        VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone())
        )
        .text(),
        edited
    );
}

#[test]
fn deleting_first_list_word_keeps_the_following_space_editable() {
    let source = "- one **bold** then *em*";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "• one bold then em");
    let edited = "•  bold then em";
    let update = projection.apply_edit(source, edited, 2..2).unwrap();
    assert_eq!(
        VisualProjection::from_markdown(&update.source).text(),
        edited,
        "updated source: {:?}",
        update.source
    );
    let styles = VisualProjection::from_markdown(&update.source);
    let runs = styles.runs_for(styles.text());
    assert!(runs.iter().any(|run| run.style.strong));
    assert!(runs.iter().any(|run| run.style.emphasis));
    assert_eq!(
        VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone())
        )
        .text(),
        edited
    );
    let active =
        VisualProjection::from_markdown_with_selection(&update.source, Some(update.selection));
    let next = "• X bold then em";
    let next_update = active.apply_edit(&update.source, next, 3..3).unwrap();
    assert_eq!(
        VisualProjection::from_markdown(&next_update.source).text(),
        next
    );
}

#[test]
fn replacing_first_list_word_with_space_keeps_every_space_visible() {
    for source in [
        "- one **bold** then *em*",
        "+ one **bold** then *em*",
        "* one **bold** then *em*",
        "1. one **bold** then *em*",
        "- [ ] one **bold** then *em*",
    ] {
        let projection = VisualProjection::from_markdown(source);
        let start_byte = projection.text().find("one").unwrap();
        let start = projection.text()[..start_byte].chars().count();
        for spaces in 1..=3 {
            let edited = projection.text().replacen("one", &" ".repeat(spaces), 1);
            let caret = start + spaces;
            let update = projection
                .apply_edit(source, &edited, caret..caret)
                .unwrap();
            assert_eq!(
                VisualProjection::from_markdown(&update.source).text(),
                edited,
                "original: {source:?}, updated source: {:?}",
                update.source
            );
        }
    }
}

#[test]
fn thematic_break_does_not_replay_tabs_consumed_by_its_source_range() {
    let source = "x\n**\t*\t\t\t\t\t\t";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "x\n────────────────");
    let positions = (0..=projection.text().chars().count())
        .map(|point| projection.source_char_range(source, point..point).start)
        .collect::<Vec<_>>();
    assert!(
        positions.windows(2).all(|pair| pair[0] <= pair[1]),
        "positions={positions:?}"
    );
    assert_eq!(positions.last().copied(), Some(source.chars().count()));
}

#[test]
fn same_line_nested_list_markers_map_to_their_own_source_prefix() {
    let source = "* * x";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "• \n• x");
    let child = projection.text().chars().position(|ch| ch == '\n').unwrap() + 1;
    assert_eq!(projection.source_char_range(source, child..child), 2..2);
    let positions = (0..=projection.text().chars().count())
        .map(|point| projection.source_char_range(source, point..point).start)
        .collect::<Vec<_>>();
    assert!(
        positions.windows(2).all(|pair| pair[0] <= pair[1]),
        "positions={positions:?}"
    );
}

#[test]
fn editing_a_nested_list_marker_preserves_the_parent_marker() {
    let source = "* * x";
    let projection = VisualProjection::from_markdown(source);
    // Typing before the child's marker must not insert before the parent item.
    let update = projection.apply_edit(source, "• \nX• x", 4..4).unwrap();
    assert_eq!(update.source, "* X* x");
    assert_eq!(update.selection, 3..3);
    let update = projection.apply_edit(source, "• \nx", 3..3).unwrap();
    assert_eq!(update.source, "* x");
    assert_eq!(update.selection, 2..2);
    assert_eq!(
        VisualProjection::from_markdown(&update.source).text(),
        "• x"
    );
    let update = projection.apply_edit(source, "• \n新🙂x", 5..5).unwrap();
    assert_eq!(update.source, "* 新🙂x");
    assert_eq!(update.selection, 4..4);
    assert_eq!(
        VisualProjection::from_markdown(&update.source).text(),
        "• 新🙂x"
    );
}

#[test]
fn nested_list_prefixes_preserve_unconsumed_quote_and_indentation() {
    for (source, expected) in [
        ("* * ", "• \n• "),
        ("* * * 中", "• \n• \n• 中"),
        ("  * 中", "  • 中"),
        ("> * * 中", "│ • \n• 中"),
        ("* > * 中", "• \n│ • 中"),
        ("* > > * 中", "• \n│ │ • 中"),
        ("1. 2. 中", "1. \n2. 中"),
        ("* * [x] 中", "• \n☑ 中"),
        ("* [ ] 甲\n  * 中", "☐ 甲\n  • 中"),
        ("* 甲\n  * 乙", "• 甲\n  • 乙"),
        ("* 甲\r\n  * 乙", "• 甲\n  • 乙"),
        ("* 甲\r  * 乙", "• 甲\n  • 乙"),
        ("> * 甲\n>   * 乙", "│ • 甲\n│   • 乙"),
    ] {
        let projection = VisualProjection::from_markdown(source);
        assert_eq!(projection.text(), expected, "source={source:?}");
        let positions = (0..=projection.text().chars().count())
            .map(|point| projection.source_char_range(source, point..point).start)
            .collect::<Vec<_>>();
        assert!(
            positions.windows(2).all(|pair| pair[0] <= pair[1]),
            "source={source:?}, positions={positions:?}"
        );
        let mut edited = projection.text().to_owned();
        edited.push('🙂');
        let cursor = edited.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, format!("{source}🙂"));
        assert_eq!(
            update.selection,
            update.source.chars().count()..update.source.chars().count()
        );
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited
        );
    }
}

#[test]
fn bare_cr_line_break_does_not_repeat_previous_indentation() {
    for (source, expected) in [
        (" a\rb", " a\nb"),
        (" a\r\nb", " a\nb"),
        (" a\nb", " a\nb"),
        (" \rb", "\nb"),
        (" 甲\r乙", " 甲\n乙"),
        (" \u{3}\rV+", " \u{3}\nV+"),
    ] {
        let projection = VisualProjection::from_markdown(source);
        assert_eq!(projection.text(), expected, "source={source:?}");
        let positions = (0..=projection.text().chars().count())
            .map(|index| projection.source_char_range(source, index..index).start)
            .collect::<Vec<_>>();
        assert!(
            positions.windows(2).all(|pair| pair[0] <= pair[1]),
            "source={source:?} positions={positions:?}"
        );
    }
}

#[test]
fn inserting_after_bare_cr_edits_the_second_source_line() {
    let source = " a\rb";
    let projection = VisualProjection::from_markdown(source);
    let at = projection.text().find('\n').unwrap() + 1;
    let mut edited = projection.text().to_owned();
    edited.insert(at, 'X');
    let cursor = edited[..at + 1].chars().count();
    let update = projection
        .apply_edit(source, &edited, cursor..cursor)
        .unwrap();
    assert_eq!(update.source, " a\rXb");
    assert_eq!(update.selection, 4..4);
    assert_eq!(
        VisualProjection::from_markdown(&update.source).text(),
        " a\nXb"
    );
}

#[test]
fn editing_across_bare_cr_preserves_source_ranges_and_unicode() {
    for (source, edited, selection, expected_source) in [
        (" a\rb", " a\n中", 3..4, " a\r中"),
        (" a\rb", " a\n", 3..3, " a\r"),
        (" a\rb", " ab", 2..2, " ab"),
        (" 甲\r乙", " 甲\n🙂乙", 4..4, " 甲\r🙂乙"),
    ] {
        let projection = VisualProjection::from_markdown(source);
        let update = projection
            .apply_edit(source, edited, selection.clone())
            .unwrap();
        assert_eq!(update.source, expected_source, "source={source:?}");
        assert_eq!(update.selection, selection, "source={source:?}");
        let reparsed = VisualProjection::from_markdown(&update.source);
        assert_eq!(reparsed.text(), edited, "source={source:?}");
        assert_eq!(
            reparsed.visual_char_range(&update.source, update.selection),
            selection
        );
    }
}

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
