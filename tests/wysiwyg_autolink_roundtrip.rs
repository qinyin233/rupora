use rupora::wysiwyg::VisualProjection;

fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

#[test]
fn editing_an_autolink_does_not_expose_or_hide_its_visible_text() {
    let source = "a <https://x.test> z";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "a https://x.test z");

    let edited = "a 新ttps://x.test z";
    let update = projection.apply_edit(source, edited, 3..3).unwrap();
    assert_eq!(
        VisualProjection::from_markdown(&update.source).text(),
        edited,
        "source after edit: {:?}",
        update.source
    );
}

#[test]
fn selecting_visible_autolink_text_keeps_the_link_delimiters_hidden() {
    let source = "a <https://x.test> z";
    let active = VisualProjection::from_markdown_with_selection(source, Some(3..4));
    assert_eq!(active.text(), "a https://x.test z");
}

#[test]
fn short_autolink_edits_keep_visible_text_and_valid_links() {
    let mut failures = Vec::new();
    for source in ["a <https://x.test> z", "a <user@x.test> z"] {
        let projection = VisualProjection::from_markdown(source);
        let visual = projection.text();
        let length = visual.chars().count();
        for start in 0..length {
            for end in start + 1..=(start + 3).min(length) {
                for replacement in ["", "新", " ", "x"] {
                    let mut edited = visual.to_owned();
                    edited.replace_range(byte_at(visual, start)..byte_at(visual, end), replacement);
                    if edited == visual {
                        continue;
                    }
                    let cursor = start + replacement.chars().count();
                    let Some(update) = projection.apply_edit(source, &edited, cursor..cursor)
                    else {
                        failures.push(format!("no update: source={source:?}, range={start}..{end}, replacement={replacement:?}"));
                        continue;
                    };
                    let actual = VisualProjection::from_markdown(&update.source);
                    if actual.text() != edited {
                        failures.push(format!(
                            "source={source:?}, range={start}..{end}, replacement={replacement:?}, expected={edited:?}, actual={:?}, new_source={:?}",
                            actual.text(), update.source
                        ));
                    } else if actual.visual_char_range(&update.source, update.selection.clone())
                        != (cursor..cursor)
                    {
                        failures.push(format!(
                            "cursor mismatch: source={source:?}, range={start}..{end}, replacement={replacement:?}, cursor={cursor}, actual_cursor={:?}, new_source={:?}, selected={:?}",
                            actual.visual_char_range(&update.source, update.selection.clone()), update.source, update.selection
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn valid_autolink_edit_remains_a_link() {
    for (source, edited, expected_source) in [
        (
            "a <https://x.test> z",
            "a https://y.test z",
            "a <https://y.test> z",
        ),
        ("a <user@x.test> z", "a user@y.test z", "a <user@y.test> z"),
    ] {
        let projection = VisualProjection::from_markdown(source);
        let cursor = edited[..edited.find('y').unwrap()].chars().count() + 1;
        let update = projection
            .apply_edit(source, edited, cursor..cursor)
            .unwrap();
        assert_eq!(update.source, expected_source);
        let actual = VisualProjection::from_markdown(&update.source);
        assert_eq!(actual.text(), edited);
        assert!(
            actual
                .runs_for(actual.text())
                .iter()
                .any(|run| run.style.link)
        );
    }
}
