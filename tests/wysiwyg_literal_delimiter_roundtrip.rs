use rupora::wysiwyg::{VisualProjection, VisualStyle};

fn styles(projection: &VisualProjection) -> Vec<VisualStyle> {
    projection
        .runs_for(projection.text())
        .into_iter()
        .flat_map(|run| std::iter::repeat_n(run.style, run.range.len()))
        .collect()
}

#[test]
fn whitespace_next_to_literal_underscore_keeps_text_style_and_caret() {
    for source in [
        "_a_b_",
        "__a_b__",
        "_甲_乙🙂_",
        "__甲_乙🙂__",
        "_**a_b**_",
        "**_甲_乙🙂_**",
        "__*a_b*__",
        "*__甲_乙🙂__*",
        "_a__b_",
        "__甲__乙🙂__",
        "_[a_b](u_v)_",
        "*a_b*",
        "a_b",
        "_a\\_b_",
        "_a&#95;b_",
        "*`a_b`*",
    ] {
        let before = VisualProjection::from_markdown(source);
        let chars: Vec<_> = before.text().chars().collect();
        let old_styles = styles(&before);
        let underscore = chars.iter().position(|&ch| ch == '_').unwrap();
        let underscore_end = underscore
            + chars[underscore..]
                .iter()
                .take_while(|&&ch| ch == '_')
                .count();
        for position in underscore..=underscore_end {
            for whitespace in [" ", "  ", "\t", "\u{a0}"] {
                let edited = chars[..position].iter().collect::<String>()
                    + whitespace
                    + &chars[position..].iter().collect::<String>();
                let caret = position + whitespace.chars().count();
                let update = before.apply_edit(source, &edited, caret..caret).unwrap();
                let after = VisualProjection::from_markdown(&update.source);
                assert_eq!(after.text(), edited, "{source:?}, {update:?}");
                assert_eq!(
                    after.visual_char_range(&update.source, update.selection),
                    caret..caret,
                    "{source:?}, output={:?}",
                    update.source
                );
                let new_styles = styles(&after);
                assert_eq!(new_styles[..position], old_styles[..position]);
                assert_eq!(new_styles[caret..], old_styles[position..]);

                let mut continued = edited.clone();
                let byte = continued.char_indices().nth(caret).unwrap().0;
                continued.insert_str(byte, "新🙂");
                let next = after
                    .apply_edit(&update.source, &continued, caret + 2..caret + 2)
                    .unwrap();
                let next_projection = VisualProjection::from_markdown(&next.source);
                assert_eq!(next_projection.text(), continued);
                assert_eq!(
                    next_projection.visual_char_range(&next.source, next.selection),
                    caret + 2..caret + 2
                );
                let next_styles = styles(&next_projection);
                assert_eq!(next_styles[..position], old_styles[..position]);
                assert_eq!(next_styles[caret + 2..], old_styles[position..]);
            }
        }
    }
}

#[test]
fn typing_markdown_closers_still_creates_formatting() {
    for (source, closer, strong) in [
        ("_a_b", "_", false),
        ("__a_b", "__", true),
        ("*a_b", "*", false),
    ] {
        let before = VisualProjection::from_markdown(source);
        let edited = before.text().to_owned() + closer;
        let caret = edited.chars().count();
        let update = before.apply_edit(source, &edited, caret..caret).unwrap();
        assert_eq!(update.source, format!("{source}{closer}"));
        let after = VisualProjection::from_markdown(&update.source);
        assert_eq!(after.text(), "a_b");
        assert!(
            styles(&after)
                .iter()
                .all(|style| { if strong { style.strong } else { style.emphasis } })
        );
    }
}
