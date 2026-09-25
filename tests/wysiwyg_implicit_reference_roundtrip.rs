use rupora::wysiwyg::VisualProjection;

fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

#[test]
fn editing_collapsed_reference_label_keeps_visible_text_and_target() {
    let source = "a [label][] z\n\n[label]: https://x.test";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a label z");

    let edited = "a 新abel z";
    let update = projection.apply_edit(source, edited, 3..3).unwrap();
    let actual = VisualProjection::from_markdown(&update.source);
    assert_eq!(
        actual.text(),
        edited,
        "source after edit: {:?}",
        update.source
    );
    assert!(
        actual
            .runs_for(actual.text())
            .iter()
            .any(|run| run.style.link)
    );
}

#[test]
fn editing_shortcut_reference_label_keeps_visible_text_and_target() {
    let source = "a [label] z\n\n[label]: https://x.test";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a label z");

    let edited = "a 新abel z";
    let update = projection.apply_edit(source, edited, 3..3).unwrap();
    let actual = VisualProjection::from_markdown(&update.source);
    assert_eq!(
        actual.text(),
        edited,
        "source after edit: {:?}",
        update.source
    );
    assert!(
        actual
            .runs_for(actual.text())
            .iter()
            .any(|run| run.style.link)
    );
}

#[test]
fn selecting_implicit_reference_label_keeps_syntax_hidden() {
    for source in [
        "a [label][] z\n\n[label]: https://x.test",
        "a [label] z\n\n[label]: https://x.test",
    ] {
        let active = VisualProjection::from_markdown_with_selection(source, Some(3..4));
        assert_eq!(active.text(), "a label z");
    }
}

#[test]
fn short_edits_across_implicit_reference_edges_keep_visible_text_and_caret() {
    for source in [
        "a [label][] z\n\n[label]: https://x.test",
        "a [label] z\n\n[label]: https://x.test",
    ] {
        let projection = VisualProjection::from_markdown(source);
        let visual = projection.text();
        for start in 0..7 {
            for end in start + 1..=(start + 3).min(7) {
                for replacement in ["", "新", " ", "x"] {
                    let mut edited = visual.to_owned();
                    edited.replace_range(byte_at(visual, start)..byte_at(visual, end), replacement);
                    if edited == visual {
                        continue;
                    }
                    let cursor = start + replacement.chars().count();
                    let update = projection
                        .apply_edit(source, &edited, cursor..cursor)
                        .unwrap();
                    let actual = VisualProjection::from_markdown(&update.source);
                    assert_eq!(
                        actual.text(),
                        edited,
                        "source={source:?}, edit={start}..{end} -> {replacement:?}, new_source={:?}",
                        update.source
                    );
                    assert_eq!(
                        actual.visual_char_range(&update.source, update.selection),
                        cursor..cursor,
                        "source={source:?}, edit={start}..{end} -> {replacement:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn explicit_reference_and_inline_link_are_not_rewritten() {
    for source in [
        "a [label][ref] z\n\n[ref]: https://x.test",
        "a [label](https://x.test) z",
    ] {
        let projection = VisualProjection::from_markdown(source);
        let edited = projection.text().replacen("label", "新abel", 1);
        let update = projection.apply_edit(source, &edited, 3..3).unwrap();
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited
        );
        assert!(!update.source.contains("[label][label]"));
    }
}

#[test]
fn formatting_inside_implicit_reference_label_keeps_original_lookup_id() {
    for source in [
        "a [*label*][] z\n\n[*label*]: https://x.test",
        "a [*label*] z\n\n[*label*]: https://x.test",
    ] {
        let projection = VisualProjection::from_markdown(source);
        assert_eq!(projection.text(), "a label z");
        let edited = "a 新abel z";
        let update = projection.apply_edit(source, edited, 3..3).unwrap();
        let actual = VisualProjection::from_markdown(&update.source);
        assert_eq!(actual.text(), edited, "new source: {:?}", update.source);
        assert!(
            actual
                .runs_for(actual.text())
                .iter()
                .any(|run| run.style.link)
        );
        assert!(update.source.contains("[*label*]"));
    }
}
