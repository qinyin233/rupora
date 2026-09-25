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
