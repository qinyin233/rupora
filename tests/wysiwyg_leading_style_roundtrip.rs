use rupora::wysiwyg::{VisualProjection, VisualStyle};

fn inline_styles(projection: &VisualProjection) -> Vec<VisualStyle> {
    projection
        .runs_for(projection.text())
        .into_iter()
        .flat_map(|run| {
            let mut style = run.style;
            // A retained space may become a structural indentation marker.
            // Its inline formatting must still survive the edit unchanged.
            style.marker = false;
            std::iter::repeat_n(style, run.range.len())
        })
        .collect()
}

#[test]
fn plain_edits_preserve_surviving_inline_styles_and_caret() {
    for source in [
        "A **甲🙂乙** Z",
        "A __甲🙂乙__ Z",
        "A *甲🙂乙* Z",
        "A _甲🙂乙_ Z",
        "A ~~甲🙂乙~~ Z",
        "A ***甲🙂乙*** Z",
        "A __甲_🙂_乙__ Z",
        "A *a **b** c* Z",
        "A **a *b* c** Z",
        "A ~~a *b* c~~ Z",
        "A *a* **b** _c_ Z",
        "A **a** *b* ~~c~~ Z",
        "A __a__ _b_ Z",
        "A _a_ __b__ Z",
        "A _a b_ Z",
        "A *a b* Z",
        "A ~~a b~~ Z",
        "A ***a b*** Z",
        "A **x&fjlig;y** Z",
        "A [**x&fjlig;y**](url) Z",
    ] {
        let before = VisualProjection::from_markdown(source);
        let chars: Vec<_> = before.text().chars().collect();
        let old_styles = inline_styles(&before);
        for start in 0..=chars.len() {
            for end in start..=chars.len() {
                for replacement in ["新🙂", "", " "] {
                    let edited = chars[..start].iter().collect::<String>()
                        + replacement
                        + &chars[end..].iter().collect::<String>();
                    if edited == before.text() {
                        continue;
                    }
                    let caret = start + replacement.chars().count();
                    let update = before.apply_edit(source, &edited, caret..caret).unwrap();
                    let after = VisualProjection::from_markdown(&update.source);
                    let context = format!(
                        "{source:?}, range={start}..{end}, replacement={replacement:?}, output={:?}",
                        update.source
                    );
                    assert_eq!(after.text(), edited, "{context}");
                    assert_eq!(
                        after.visual_char_range(&update.source, update.selection),
                        caret..caret,
                        "{context}"
                    );
                    let new_styles = inline_styles(&after);
                    assert_eq!(new_styles[..start], old_styles[..start], "{context}");
                    assert_eq!(new_styles[caret..], old_styles[end..], "{context}");
                }
            }
        }
    }
}

#[test]
fn leading_styles_belong_to_the_enclosing_context() {
    let marker = VisualStyle {
        marker: true,
        ..VisualStyle::default()
    };
    for (source, range, expected) in [
        (" **a**", 0..1, marker),
        (" ~~a~~", 0..1, marker),
        (" [a](u)", 0..1, marker),
        (" _**a**_", 0..1, marker),
        (" **_a_**", 0..1, marker),
        (
            " ### **a**",
            0..1,
            VisualStyle {
                heading: 3,
                ..marker
            },
        ),
        (
            "> **a**",
            0..2,
            VisualStyle {
                quote: true,
                ..marker
            },
        ),
        (
            ">  **a**",
            0..3,
            VisualStyle {
                quote: true,
                ..marker
            },
        ),
        (
            ">\t**a**",
            0..2,
            VisualStyle {
                quote: true,
                ..marker
            },
        ),
        (
            "**a\n  *b* c**",
            2..4,
            VisualStyle {
                strong: true,
                ..marker
            },
        ),
        (
            "_a\n **b** c_",
            2..3,
            VisualStyle {
                emphasis: true,
                ..marker
            },
        ),
        (
            "~~a\n _b_ c~~",
            2..3,
            VisualStyle {
                strikethrough: true,
                ..marker
            },
        ),
        (
            "**<span\n a=x>*b*</span>**",
            0..1,
            VisualStyle {
                strong: true,
                ..marker
            },
        ),
        (
            "**<!--\n x -->*b***",
            0..1,
            VisualStyle {
                strong: true,
                ..marker
            },
        ),
        (
            "` a`",
            0..1,
            VisualStyle {
                code: true,
                ..VisualStyle::default()
            },
        ),
    ] {
        let newlines: &[&str] = if source.contains('\n') {
            &["\n", "\r\n", "\r"]
        } else {
            &["\n"]
        };
        for newline in newlines {
            let source = source.replace('\n', newline);
            let projection = VisualProjection::from_markdown(&source);
            let styles: Vec<_> = projection
                .runs_for(projection.text())
                .into_iter()
                .flat_map(|run| std::iter::repeat_n(run.style, run.range.len()))
                .collect();
            assert!(
                styles[range.clone()].iter().all(|style| *style == expected),
                "source={source:?}, visible={:?}, expected={expected:?}, actual={:?}",
                projection.text(),
                &styles[range.clone()]
            );
        }
    }
}
