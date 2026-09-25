use rupora::wysiwyg::VisualProjection;

fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

#[test]
fn typing_plain_unicode_inside_editable_visual_text_survives_reprojection() {
    let sources = [
        "ordinary text",
        "a *emphasis* tail",
        "a **strong** tail",
        "a `code` tail",
        "a `line\nbreak` tail",
        "a [link](notes.md) tail",
        "a ![image](asset.png) tail",
        "a &fjlig; tail",
        "a &#32; tail",
        "a [^note] tail",
        "a <span>html</span> tail",
        "# heading text",
        "- list item",
        "> quoted text",
        "| left | right |\n| --- | --- |\n| cell | value |",
        "a $x+y$ tail",
    ];
    let mut failures = Vec::new();
    for source in sources {
        let projection = VisualProjection::from_markdown(source);
        let visual = projection.text();
        let runs = projection.runs_for(visual);
        for point in 0..=visual.chars().count() {
            if visual.chars().nth(point.saturating_sub(1)) == Some('\n')
                || visual.chars().nth(point) == Some('\n')
            {
                continue;
            }
            if !runs.iter().any(|run| {
                !run.style.marker
                    && !run.style.footnote
                    && run.range.start < point
                    && point < run.range.end
            }) {
                continue;
            }
            let mut edited = visual.to_owned();
            edited.insert(byte_at(visual, point), '中');
            let update = projection
                .apply_edit(source, &edited, point + 1..point + 1)
                .unwrap();
            let actual = VisualProjection::from_markdown(&update.source);
            if actual.text() != edited {
                failures.push(format!(
                    "source={source:?}, point={point}, expected={edited:?}, actual={:?}, new_source={:?}",
                    actual.text(), update.source
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn typing_on_the_hidden_table_divider_keeps_the_table_and_input() {
    for (source, typed, expected_source) in [
        (
            "| left | right |\n| --- | --- |\n| cell | value |",
            "中",
            "| left | right |\n| --- | --- |\n| 中cell | value |",
        ),
        (
            "left | right\n--- | ---\ncell | value",
            "🙂",
            "left | right\n--- | ---\n🙂cell | value",
        ),
        (
            "| left | right |\n| --- | --- |\n|  | value |",
            "中",
            "| left | right |\n| --- | --- |\n| 中 | value |",
        ),
        (
            "| left | right |\r\n| --- | --- |\r\n| cell | value |",
            "中",
            "| left | right |\r\n| --- | --- |\r\n| 中cell | value |",
        ),
        (
            "| left | right |\n| --- | --- |\n| cell | value |",
            " ",
            "| left | right |\n| --- | --- |\n| &#32;cell | value |",
        ),
    ] {
        let projection = VisualProjection::from_markdown(source);
        let spacer = projection.text().find("\n\n").unwrap();
        let point = projection.text()[..spacer].chars().count() + 1;
        let mut edited = projection.text().to_owned();
        edited.insert_str(byte_at(&edited, point), typed);
        let requested_cursor = point + typed.chars().count();
        let update = projection
            .apply_edit(source, &edited, requested_cursor..requested_cursor)
            .unwrap();
        assert_eq!(update.source, expected_source, "source: {source:?}");

        let active = VisualProjection::from_markdown_with_selection(
            &update.source,
            Some(update.selection.clone()),
        );
        let expected_visual = projection
            .text()
            .replacen("\n\n", &format!("\n\n{typed}"), 1);
        assert_eq!(active.text(), expected_visual, "source: {source:?}");
        let expected_cursor = point + 1 + typed.chars().count();
        assert_eq!(
            active.visual_char_range(&update.source, update.selection),
            expected_cursor..expected_cursor,
            "source: {source:?}"
        );
    }
}

#[test]
fn selected_space_inserted_among_repeated_table_spaces_keeps_an_ordered_selection() {
    let source = "X | b |\n| - | - |\n| x | y |";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "X  │  b\n\nx  │  y");
    let edited = "X   │  b\n\nx  │  y";
    let update = projection.apply_edit(source, edited, 2..3).unwrap();
    assert!(update.selection.start <= update.selection.end);
    let active = VisualProjection::from_markdown_with_selection(
        &update.source,
        Some(update.selection.clone()),
    );
    let selected_source = &update.source[byte_at(&update.source, update.selection.start)
        ..byte_at(&update.source, update.selection.end)];
    assert_eq!(selected_source, "&#32;");
    assert_eq!(active.text(), edited);
    let visible_selection = active.visual_char_range(&update.source, update.selection);
    assert_eq!(visible_selection.len(), 1);
    assert_eq!(
        &active.text()[byte_at(active.text(), visible_selection.start)
            ..byte_at(active.text(), visible_selection.end)],
        " "
    );
}
