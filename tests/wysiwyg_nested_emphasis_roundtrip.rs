use rupora::wysiwyg::{VisualProjection, VisualStyle};

fn styles(projection: &VisualProjection) -> Vec<VisualStyle> {
    let mut styles = vec![VisualStyle::default(); projection.text().chars().count()];
    for run in projection.runs_for(projection.text()) {
        styles[run.range].fill(run.style);
    }
    styles
}

#[test]
fn editing_nested_emphasis_boundaries_preserves_text_styles_and_caret() {
    for source in [
        "A __a_!_b__ Z",
        "A __甲_🙂_乙__ Z",
        "A __甲_🙂_甲__ Z",
        "A **甲*🙂*乙** Z",
        "A _甲🙂乙_ Z",
        "A __甲🙂乙__ Z",
    ] {
        let before = VisualProjection::from_markdown(source);
        let chars: Vec<_> = before.text().chars().collect();
        let before_styles = styles(&before);
        for (range, replacement) in [
            (0..3, "新🙂"),
            (0..4, "新🙂"),
            (1..3, "新🙂"),
            (1..3, ""),
            (1..4, "新🙂"),
            (3..6, "新🙂"),
            (4..6, "新🙂"),
            (4..6, ""),
        ] {
            let edited = chars[..range.start].iter().collect::<String>()
                + replacement
                + &chars[range.end..].iter().collect::<String>();
            let caret = range.start + replacement.chars().count();
            let update = before.apply_edit(source, &edited, caret..caret).unwrap();
            let after = VisualProjection::from_markdown_with_selection(
                &update.source,
                Some(update.selection.clone()),
            );
            assert_eq!(
                after.text(),
                edited,
                "source={source:?}, range={range:?}, update={update:?}"
            );
            assert_eq!(
                after.visual_char_range(&update.source, update.selection),
                caret..caret
            );
            let after_styles = styles(&after);
            assert_eq!(
                after_styles[..range.start],
                before_styles[..range.start],
                "prefix source={source:?}, range={range:?}, output={:?}",
                update.source
            );
            assert_eq!(
                after_styles[caret..],
                before_styles[range.end..],
                "suffix source={source:?}, range={range:?}, output={:?}",
                update.source
            );
        }
    }
}
