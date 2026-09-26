use rupora::wysiwyg::{VisualProjection, VisualStyle};

fn styles(projection: &VisualProjection) -> Vec<VisualStyle> {
    let mut styles = vec![VisualStyle::default(); projection.text().chars().count()];
    for run in projection.runs_for(projection.text()) {
        styles[run.range].fill(run.style);
    }
    styles
}

fn assert_whitespace_edit(source: &str, range: std::ops::Range<usize>, replacement: &str) {
    let before = VisualProjection::from_markdown(source);
    let chars: Vec<_> = before.text().chars().collect();
    let before_styles = styles(&before);
    let edited = chars[..range.start].iter().collect::<String>()
        + replacement
        + &chars[range.end..].iter().collect::<String>();
    let caret = range.start + replacement.chars().count();
    let update = before.apply_edit(source, &edited, caret..caret).unwrap();
    let after = VisualProjection::from_markdown(&update.source);
    assert_eq!(
        after.text(),
        edited,
        "source={source:?}, range={range:?}, output={:?}",
        update.source
    );
    assert_eq!(
        after.visual_char_range(&update.source, update.selection),
        caret..caret
    );
    let after_styles = styles(&after);
    assert_eq!(
        after_styles[..range.start],
        before_styles[..range.start],
        "prefix {source:?}, {range:?}"
    );
    assert_eq!(
        after_styles[caret..],
        before_styles[range.end..],
        "suffix {source:?}, {range:?}"
    );
}

#[test]
fn inserting_spaces_at_nested_emphasis_boundaries_preserves_text_styles_and_caret() {
    for source in ["__a_!_b__", "__甲_🙂_乙__", "**a*!*b**", "**甲*🙂*乙**"] {
        for point in 0..=3 {
            for whitespace in [" ", "  ", "\t", "\u{a0}"] {
                assert_whitespace_edit(source, point..point, whitespace);
            }
        }
    }
}

#[test]
fn retained_nested_delimiter_runs_keep_text_styles_and_caret() {
    for delimiter in ["***", "___", "_**", "**_", "~~*", "*~~"] {
        let closer: String = delimiter.chars().rev().collect();
        for text in ["a b", "甲 🙂"] {
            let source = format!("A {delimiter}{text}{closer} Z");
            assert_eq!(
                VisualProjection::from_markdown(&source).text(),
                format!("A {text} Z")
            );
            for (range, replacement) in [(1..3, ""), (3..6, " "), (4..6, "")] {
                assert_whitespace_edit(&source, range, replacement);
            }
        }
    }
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
