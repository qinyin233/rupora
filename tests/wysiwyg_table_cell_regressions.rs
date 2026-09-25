use rupora::wysiwyg::VisualProjection;

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
