use rupora::wysiwyg::VisualProjection;

fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

#[test]
fn typing_inside_a_visual_table_separator_keeps_both_columns_and_the_caret() {
    let source = "| a | b |\n| - | - |\n| c | d |";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a  │  b\n\nc  │  d");

    for (index, expected_source, expected_cursor) in [
        (2, "| aX| b |\n| - | - |\n| c | d |", 2),
        (3, "| aX| b |\n| - | - |\n| c | d |", 2),
        (4, "| a |Xb |\n| - | - |\n| c | d |", 7),
        (5, "| a |Xb |\n| - | - |\n| c | d |", 7),
        (11, "| a | b |\n| - | - |\n| cX| d |", 11),
        (12, "| a | b |\n| - | - |\n| cX| d |", 11),
        (13, "| a | b |\n| - | - |\n| c |Xd |", 16),
        (14, "| a | b |\n| - | - |\n| c |Xd |", 16),
    ] {
        let mut edited = projection.text().to_owned();
        edited.insert(byte_at(&edited, index), 'X');
        let update = projection
            .apply_edit(source, &edited, index + 1..index + 1)
            .unwrap();
        assert_eq!(update.source, expected_source, "index={index}");
        assert_eq!(
            update.source.matches('|').count(),
            source.matches('|').count()
        );
        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert_eq!(
            active.visual_char_range(&update.source, update.selection.clone()),
            expected_cursor..expected_cursor,
            "index={index}"
        );

        let mut next = active.text().to_owned();
        next.insert(byte_at(&next, expected_cursor), 'Y');
        let follow_up = active
            .apply_edit(
                &update.source,
                &next,
                expected_cursor + 1..expected_cursor + 1,
            )
            .unwrap();
        assert!(follow_up.source.contains("XY"), "index={index}");
        assert_eq!(
            follow_up.source.matches('|').count(),
            source.matches('|').count(),
            "index={index}"
        );
        assert_eq!(
            VisualProjection::from_markdown(&follow_up.source).text(),
            next
        );
    }
}

#[test]
fn typing_space_inside_a_visual_table_separator_stays_visible() {
    let source = "| a | b |\n| - | - |\n| c | d |";
    let projection = VisualProjection::from_markdown(source);
    for (index, expected_source, expected_visual, expected_cursor) in [
        (
            2,
            "| a&#32;| b |\n| - | - |\n| c | d |",
            "a   │  b\n\nc  │  d",
            2,
        ),
        (
            3,
            "| a&#32;| b |\n| - | - |\n| c | d |",
            "a   │  b\n\nc  │  d",
            2,
        ),
        (
            4,
            "| a |&#32;b |\n| - | - |\n| c | d |",
            "a  │   b\n\nc  │  d",
            7,
        ),
        (
            5,
            "| a |&#32;b |\n| - | - |\n| c | d |",
            "a  │   b\n\nc  │  d",
            7,
        ),
        (
            11,
            "| a | b |\n| - | - |\n| c&#32;| d |",
            "a  │  b\n\nc   │  d",
            11,
        ),
        (
            12,
            "| a | b |\n| - | - |\n| c&#32;| d |",
            "a  │  b\n\nc   │  d",
            11,
        ),
        (
            13,
            "| a | b |\n| - | - |\n| c |&#32;d |",
            "a  │  b\n\nc  │   d",
            16,
        ),
        (
            14,
            "| a | b |\n| - | - |\n| c |&#32;d |",
            "a  │  b\n\nc  │   d",
            16,
        ),
    ] {
        let mut edited = projection.text().to_owned();
        edited.insert(byte_at(&edited, index), ' ');
        let update = projection
            .apply_edit(source, &edited, index + 1..index + 1)
            .unwrap();
        assert_eq!(update.source, expected_source, "index={index}");
        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert_eq!(active.text(), expected_visual, "index={index}");
        assert_eq!(
            active.visual_char_range(&update.source, update.selection.clone()),
            expected_cursor..expected_cursor,
            "index={index}"
        );
        let mut next = active.text().to_owned();
        next.insert(byte_at(&next, expected_cursor), 'X');
        let follow_up = active
            .apply_edit(
                &update.source,
                &next,
                expected_cursor + 1..expected_cursor + 1,
            )
            .unwrap();
        assert_eq!(
            VisualProjection::from_markdown(&follow_up.source).text(),
            next,
            "index={index}"
        );
    }
}

#[test]
fn unicode_and_space_typed_at_a_table_separator_keep_utf8_caret_boundaries() {
    let source = "| 中🙂 | 后 |\n| --- | --- |";
    let projection = VisualProjection::from_markdown(source);
    let separator = projection.text().chars().position(|ch| ch == '│').unwrap();
    let mut edited = projection.text().to_owned();
    edited.insert_str(byte_at(&edited, separator), "新 🙂");
    let cursor = separator + "新 🙂".chars().count();
    let update = projection
        .apply_edit(source, &edited, cursor..cursor)
        .unwrap();
    assert_eq!(update.source, "| 中🙂新&#32;🙂| 后 |\n| --- | --- |");
    let active = VisualProjection::from_markdown_with_selection(
        &update.source,
        Some(update.selection.clone()),
    );
    assert_eq!(active.text(), "中🙂新 🙂  │  后");
    assert_eq!(
        active.visual_char_range(&update.source, update.selection),
        5..5
    );
}

#[test]
fn deleting_single_header_cell_character_keeps_the_table_columns() {
    let source = "| A | **B** |\n| --- | --- |\n| x | y |";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "A  │  B\n\nx  │  y");
    let edited = "  │  B\n\nx  │  y";
    let update = projection.apply_edit(source, edited, 0..0).unwrap();
    assert_eq!(update.source, "|  | **B** |\n| --- | --- |\n| x | y |");
    let reparsed = VisualProjection::from_markdown(&update.source);
    assert_eq!(reparsed.text(), edited);
    assert!(
        reparsed
            .runs_for(reparsed.text())
            .iter()
            .any(|run| run.style.table)
    );
    let active = VisualProjection::from_markdown_with_selection(
        &update.source,
        Some(update.selection.clone()),
    );
    assert_eq!(active.text(), edited);
    assert_eq!(
        active.visual_char_range(&update.source, update.selection),
        0..0
    );
    let next = "新  │  B\n\nx  │  y";
    let next_update = active.apply_edit(&update.source, next, 1..1).unwrap();
    assert_eq!(
        next_update.source,
        "| 新 | **B** |\n| --- | --- |\n| x | y |"
    );
}

#[test]
fn deleting_single_body_cell_character_keeps_the_table_columns() {
    let source = "| A | **B** |\n| --- | --- |\n| x | y |";
    let projection = VisualProjection::from_markdown(source);
    let edited = "A  │  B\n\n  │  y";
    let update = projection.apply_edit(source, edited, 9..9).unwrap();
    assert_eq!(update.source, "| A | **B** |\n| --- | --- |\n|  | y |");
    let reparsed = VisualProjection::from_markdown(&update.source);
    assert_eq!(reparsed.text(), edited);
    assert!(
        reparsed
            .runs_for(reparsed.text())
            .iter()
            .any(|run| run.style.table)
    );
    let active = VisualProjection::from_markdown_with_selection(
        &update.source,
        Some(update.selection.clone()),
    );
    assert_eq!(active.text(), edited);
    assert_eq!(
        active.visual_char_range(&update.source, update.selection),
        9..9
    );
    let next = "A  │  B\n\n新  │  y";
    let next_update = active.apply_edit(&update.source, next, 10..10).unwrap();
    assert_eq!(
        next_update.source,
        "| A | **B** |\n| --- | --- |\n| 新 | y |"
    );
}

#[test]
fn replacing_single_table_cell_character_with_space_keeps_it_visible() {
    let source = "| A | **B** |\n| --- | --- |\n| x | y |";
    let projection = VisualProjection::from_markdown(source);
    for (edited, cursor, expected_source) in [
        (
            "   │  B\n\nx  │  y",
            1,
            "| &#32; | **B** |\n| --- | --- |\n| x | y |",
        ),
        (
            "A  │  B\n\n   │  y",
            10,
            "| A | **B** |\n| --- | --- |\n| &#32; | y |",
        ),
    ] {
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, expected_source);
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
fn replacing_each_single_table_cell_character_preserves_the_visible_edit() {
    let source = "| A | **B** |\n| --- | --- |\n| x | y |";
    let projection = VisualProjection::from_markdown(source);
    let original = projection.text();
    assert_eq!(original, "A  │  B\n\nx  │  y");
    for index in [0, 6, 9, 15] {
        for replacement in ["", " ", "新", "z"] {
            let start = original.char_indices().nth(index).unwrap().0;
            let end = original
                .char_indices()
                .nth(index + 1)
                .map_or(original.len(), |(byte, _)| byte);
            let mut edited = original.to_owned();
            edited.replace_range(start..end, replacement);
            let cursor = index + replacement.chars().count();
            let update = projection
                .apply_edit(source, &edited, cursor..cursor)
                .unwrap();
            let reparsed = VisualProjection::from_markdown(&update.source);
            assert_eq!(
                reparsed.text(),
                edited,
                "index={index} replacement={replacement:?} source={:?}",
                update.source
            );
        }
    }
}

#[test]
fn deleting_the_last_word_character_keeps_a_surviving_table_cell_space() {
    let source = "| a b | c d |\n| --- | --- |\n| e f | g h |";
    let projection = VisualProjection::from_markdown(source);
    for (edited, cursor, expected_source) in [
        (
            "a   │  c d\n\ne f  │  g h",
            2,
            "| a&#32; | c d |\n| --- | --- |\n| e f | g h |",
        ),
        (
            "a b  │  c \n\ne f  │  g h",
            10,
            "| a b | c&#32; |\n| --- | --- |\n| e f | g h |",
        ),
        (
            "a b  │  c d\n\ne f  │  g ",
            23,
            "| a b | c d |\n| --- | --- |\n| e f | g&#32; |",
        ),
    ] {
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, expected_source);
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
fn deleting_the_only_header_cell_character_keeps_caret_in_the_empty_cell() {
    let source = "| a | b |\n| - | - |\n| c | d |";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a  │  b\n\nc  │  d");

    let edited = "a  │  \n\nc  │  d";
    let update = projection.apply_edit(source, edited, 6..6).unwrap();
    assert_eq!(update.source, "| a |  |\n| - | - |\n| c | d |");
    let active = VisualProjection::from_markdown_with_selection(
        &update.source,
        Some(update.selection.clone()),
    );
    assert_eq!(active.text(), edited);
    assert_eq!(
        active.visual_char_range(&update.source, update.selection),
        6..6
    );
}

#[test]
fn typing_a_space_at_the_end_of_a_table_cell_keeps_it_visible() {
    let source = "| a | b |\n| - | - |\n| c | d |";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a  │  b\n\nc  │  d");
    for (edited, cursor, expected_source) in [
        (
            "a   │  b\n\nc  │  d",
            2,
            "| a&#32; | b |\n| - | - |\n| c | d |",
        ),
        (
            "a  │  b\n\nc  │  d ",
            17,
            "| a | b |\n| - | - |\n| c | d&#32; |",
        ),
    ] {
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, expected_source);
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
fn typing_multiple_spaces_at_a_table_cell_end_keeps_every_space() {
    let source = "| a | b |\n| - | - |\n| c | d |";
    let projection = VisualProjection::from_markdown(source);
    for count in 2..=3 {
        let edited = format!("a{}  │  b\n\nc  │  d", " ".repeat(count));
        let cursor = 1 + count;
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert_eq!(active.text(), edited, "source={:?}", update.source);
        assert_eq!(
            active.visual_char_range(&update.source, update.selection),
            cursor..cursor
        );
    }
}

#[test]
fn replacing_the_last_cell_character_preserves_preceding_spaces() {
    let source = "| a  b | c |\n| - | - |\n| d | e |";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a  b  │  c\n\nd  │  e");
    for replacement in ["", " ", "  "] {
        let edited = projection.text().replacen('b', replacement, 1);
        let cursor = 3 + replacement.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        let reparsed = VisualProjection::from_markdown(&update.source);
        assert_eq!(
            reparsed.text(),
            edited,
            "replacement={replacement:?}, source={:?}",
            update.source
        );
        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert_eq!(
            active.visual_char_range(&update.source, update.selection),
            cursor..cursor
        );
    }
}

#[test]
fn deleting_final_table_cell_character_before_a_paragraph_keeps_caret_in_cell() {
    let source = "a\n\n| x | y |\n| - | - |\n| p | q |\n\nz";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a\n\nx  │  y\n\np  │  q\n\nz");
    let edited = "a\n\nx  │  y\n\np  │  \n\nz";
    let update = projection.apply_edit(source, edited, 18..18).unwrap();
    assert_eq!(
        VisualProjection::from_markdown(&update.source).text(),
        edited
    );
    let active = VisualProjection::from_markdown_with_selection(
        &update.source,
        Some(update.selection.clone()),
    );
    assert_eq!(
        active.visual_char_range(&update.source, update.selection),
        18..18
    );
}

#[test]
fn replacing_last_column_character_with_spaces_keeps_caret_before_row_break() {
    let source = "| a | b |\n| - | - |\n| c | d |";
    let projection = VisualProjection::from_markdown(source);
    for count in 1..=2 {
        let edited = format!("a  │  {}\n\nc  │  d", " ".repeat(count));
        let cursor = 6 + count;
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert_eq!(active.text(), edited);
        assert_eq!(
            active.visual_char_range(&update.source, update.selection.clone()),
            cursor..cursor
        );
        let next = format!("a  │  {}X\n\nc  │  d", " ".repeat(count));
        let follow_up = active
            .apply_edit(&update.source, &next, cursor + 1..cursor + 1)
            .unwrap();
        assert_eq!(
            VisualProjection::from_markdown(&follow_up.source).text(),
            next
        );
    }
}
