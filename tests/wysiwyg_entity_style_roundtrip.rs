use rupora::wysiwyg::{VisualProjection, VisualStyle};

fn styles(projection: &VisualProjection) -> Vec<VisualStyle> {
    projection
        .runs_for(projection.text())
        .into_iter()
        .flat_map(|run| std::iter::repeat_n(run.style, run.range.len()))
        .collect()
}

#[test]
fn partial_entity_edits_preserve_unselected_styles() {
    for (source, range) in [
        ("x **&fjlig;y**", 0..3),
        ("x *&fjlig;y*", 0..3),
        ("x ~~&fjlig;y~~", 0..3),
        ("x[&fjlig;](u)", 0..2),
        ("x[&fjlig;][]\n\n[&fjlig;]: u", 0..2),
        ("x[&fjlig;]\n\n[&fjlig;]: u", 0..2),
        ("x [**&fjlig;y**](u)", 0..3),
        ("x [*&fjlig;y*][id]\n\n[id]: u", 0..3),
        ("x **&NotEqualTilde;乙**", 0..3),
        ("**x&fjlig;** y", 2..5),
        ("**&fjlig;y**", 0..1),
        ("**x&fjlig;** *&fjlig;y*", 2..5),
    ] {
        let before = VisualProjection::from_markdown(source);
        let chars: Vec<_> = before.text().chars().collect();
        let old_styles = styles(&before);
        for replacement in ["", "新🙂"] {
            let edited = chars[..range.start].iter().collect::<String>()
                + replacement
                + &chars[range.end..].iter().collect::<String>();
            let caret = range.start + replacement.chars().count();
            let update = before.apply_edit(source, &edited, caret..caret).unwrap();
            let after = VisualProjection::from_markdown(&update.source);
            assert_eq!(after.text(), edited, "source={source:?}, update={update:?}");
            assert_eq!(
                after.visual_char_range(&update.source, update.selection),
                caret..caret
            );
            let actual_styles = styles(&after);
            assert_eq!(
                actual_styles[..range.start],
                old_styles[..range.start],
                "prefix {source:?}, output={:?}",
                update.source
            );
            assert_eq!(
                actual_styles[caret..],
                old_styles[range.end..],
                "suffix {source:?}, output={:?}",
                update.source
            );
            let references = rupora::markdown::reference_definitions(&update.source);
            for (index, style) in actual_styles
                .iter()
                .enumerate()
                .filter(|(_, style)| style.link)
            {
                let source_char = after.source_char_range(&update.source, index..index).start;
                let byte = update
                    .source
                    .char_indices()
                    .nth(source_char)
                    .map_or(update.source.len(), |(byte, _)| byte);
                assert_eq!(
                    rupora::markdown::link_destination_with_references(
                        &update.source,
                        byte,
                        &references
                    )
                    .as_deref(),
                    Some("u"),
                    "{style:?}, {:?}",
                    update.source
                );
            }
        }
    }
}
