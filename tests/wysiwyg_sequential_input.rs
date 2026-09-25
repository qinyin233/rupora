use rupora::wysiwyg::VisualProjection;

#[test]
fn typing_between_surviving_leading_spaces_preserves_both_spaces() {
    let source = "a b";
    let first = VisualProjection::from_markdown(source)
        .apply_edit(source, "  b", 1..1)
        .unwrap();
    assert_eq!(first.source, "  b");
    let active = VisualProjection::from_markdown_with_selection(
        &first.source,
        Some(first.selection.clone()),
    );
    assert_eq!(active.text(), "  b");

    let second = active.apply_edit(&first.source, " X b", 2..2).unwrap();
    assert_eq!(second.source, " X b");
    let final_projection = VisualProjection::from_markdown_with_selection(
        &second.source,
        Some(second.selection.clone()),
    );
    assert_eq!(final_projection.text(), " X b");
    assert_eq!(
        final_projection.visual_char_range(&second.source, second.selection),
        2..2
    );
}

#[test]
fn typing_after_replacing_a_table_cell_stays_before_its_padding() {
    let source = "| a | b |\n| - | - |";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a  │  b");
    let first = projection.apply_edit(source, "新  │  b", 1..1).unwrap();
    assert_eq!(first.source, "| 新 | b |\n| - | - |");
    let active = VisualProjection::from_markdown_with_selection(
        &first.source,
        Some(first.selection.clone()),
    );

    let second = active.apply_edit(&first.source, "新X  │  b", 2..2).unwrap();
    assert_eq!(second.source, "| 新X | b |\n| - | - |");
    let final_projection = VisualProjection::from_markdown_with_selection(
        &second.source,
        Some(second.selection.clone()),
    );
    assert_eq!(final_projection.text(), "新X  │  b");
    assert_eq!(
        final_projection.visual_char_range(&second.source, second.selection),
        2..2
    );
}

#[test]
fn typing_after_deleting_first_quote_character_keeps_the_quote_and_spaces() {
    let source = "> a  b";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "│ a  b");
    let first = projection.apply_edit(source, "│   b", 2..2).unwrap();
    assert_eq!(
        VisualProjection::from_markdown(&first.source).text(),
        "│   b"
    );
    let active = VisualProjection::from_markdown_with_selection(
        &first.source,
        Some(first.selection.clone()),
    );
    assert_eq!(
        active.visual_char_range(&first.source, first.selection.clone()),
        2..2
    );
    let second = active.apply_edit(&first.source, "│ X  b", 3..3).unwrap();
    assert_eq!(
        VisualProjection::from_markdown(&second.source).text(),
        "│ X  b"
    );
}

#[test]
fn replacing_first_heading_character_keeps_surviving_spaces_visible() {
    let source = "## a  b";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a  b");
    for replacement in ["", " ", "  "] {
        let edited = format!("{replacement}  b");
        let cursor = replacement.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        assert_eq!(active.text(), edited, "source={:?}", update.source);
        assert_eq!(
            active.visual_char_range(&update.source, update.selection.clone()),
            cursor..cursor
        );
        let next = format!("{replacement}X  b");
        let follow_up = active
            .apply_edit(&update.source, &next, cursor + 1..cursor + 1)
            .unwrap();
        assert_eq!(
            VisualProjection::from_markdown(&follow_up.source).text(),
            next
        );
    }
}
