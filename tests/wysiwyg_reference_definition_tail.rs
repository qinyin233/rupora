use rupora::wysiwyg::VisualProjection;

#[test]
fn trailing_reference_definitions_do_not_create_an_editable_visual_line() {
    for source in [
        "[label][ref]\n\n[ref]: /x",
        "[label][ref]\r\n\r\n[ref]: /x\r\n",
        "[label][ref]\n\n[ref]: /x\n\n[other]: /y\n",
        "[label][ref]\n\n[ref]: /x\n[ref]: /y",
        "[label][ref]\n\n[ref]: /x   ",
        "[label][ref]\n\n[ref]: /x\n\n",
    ] {
        let projection = VisualProjection::from_markdown(source);
        assert_eq!(projection.text(), "label", "source={source:?}");
        assert!(
            projection
                .runs_for(projection.text())
                .iter()
                .any(|run| run.style.link)
        );

        let edited = "labelX";
        let update = projection.apply_edit(source, edited, 6..6).unwrap();
        let updated_projection = VisualProjection::from_markdown(&update.source);
        assert_eq!(updated_projection.text(), edited);
        assert_eq!(
            updated_projection.visual_char_range(&update.source, update.selection),
            6..6
        );
        if source.contains("[ref]: /y") {
            let references = rupora::markdown::reference_definitions(&update.source);
            let link_byte = update.source.find("label").unwrap();
            assert_eq!(
                rupora::markdown::link_destination_with_references(
                    &update.source,
                    link_byte,
                    &references
                ),
                Some("/x".into())
            );
        }
    }
}

#[test]
fn ordinary_trailing_newline_remains_editable_without_hidden_definitions() {
    assert_eq!(VisualProjection::from_markdown("[ref]: /x\n").text(), "");
    assert_eq!(VisualProjection::from_markdown("label\n").text(), "label\n");
    assert_eq!(
        VisualProjection::from_markdown("label\r\n").text(),
        "label\n"
    );
}
