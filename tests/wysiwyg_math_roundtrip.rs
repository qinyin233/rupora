use rupora::wysiwyg::VisualProjection;

fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

#[test]
fn short_replacements_across_math_edges_keep_visible_text() {
    let sources = [
        "a $x+y$ tail",
        "a $$x+y$$ tail",
        "前 $中🙂+y$ 后",
        "$$x+y$$\nnext",
        "*before $x+y$ after*",
        "a $x+y$ and $q+r$ tail",
    ];
    let mut failures = Vec::new();
    for source in sources {
        let projection = VisualProjection::from_markdown(source);
        let visual = projection.text();
        let runs = projection.runs_for(visual);
        let length = visual.chars().count();
        for start in 0..length {
            for end in start + 1..=(start + 3).min(length) {
                if visual.chars().nth(start.saturating_sub(1)) == Some('\n')
                    || visual.chars().nth(end) == Some('\n')
                    || visual[byte_at(visual, start)..byte_at(visual, end)].contains('\n')
                    || runs.iter().any(|run| {
                        (run.style.marker || run.style.footnote)
                            && run.range.start < end
                            && start < run.range.end
                    })
                {
                    continue;
                }
                for replacement in ["", "新", " ", "x"] {
                    let mut edited = visual.to_owned();
                    edited.replace_range(byte_at(visual, start)..byte_at(visual, end), replacement);
                    if edited == visual {
                        continue;
                    }
                    let cursor = start + replacement.chars().count();
                    let Some(update) = projection.apply_edit(source, &edited, cursor..cursor)
                    else {
                        failures.push(format!("no update: source={source:?}, visual={visual:?}, edited={edited:?}, range={start}..{end}, replacement={replacement:?}"));
                        continue;
                    };
                    let actual = VisualProjection::from_markdown(&update.source);
                    if actual.text() != edited {
                        failures.push(format!(
                            "source={source:?}, range={start}..{end}, replacement={replacement:?}, expected={edited:?}, actual={:?}, new_source={:?}",
                            actual.text(), update.source
                        ));
                    } else {
                        let active = VisualProjection::from_markdown_with_selection(
                            &update.source,
                            Some(update.selection.clone()),
                        );
                        if active.text() == edited
                            && active.visual_char_range(&update.source, update.selection.clone())
                                != (cursor..cursor)
                        {
                            failures.push(format!(
                                "source={source:?}, range={start}..{end}, replacement={replacement:?}, cursor={cursor}, new_source={:?}, selected={:?}",
                                update.source, update.selection
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn spaces_inserted_at_math_edges_do_not_expose_dollar_delimiters() {
    for source in ["a $x+y$ tail", "前 $中🙂+y$ 后"] {
        let projection = VisualProjection::from_markdown(source);
        let visual = projection.text();
        let content = if source.starts_with('a') {
            "x+y"
        } else {
            "中🙂+y"
        };
        let start = visual[..visual.find(content).unwrap()].chars().count();
        for point in [start, start + content.chars().count()] {
            let mut edited = visual.to_owned();
            edited.insert(byte_at(visual, point), ' ');
            let update = projection
                .apply_edit(source, &edited, point + 1..point + 1)
                .unwrap();
            assert_eq!(
                VisualProjection::from_markdown(&update.source).text(),
                edited,
                "source={source:?}, point={point}, new_source={:?}",
                update.source
            );
            let active = VisualProjection::from_markdown_with_selection(
                &update.source,
                Some(update.selection.clone()),
            );
            assert_eq!(active.text(), edited);
            assert_eq!(
                active.visual_char_range(&update.source, update.selection),
                point + 1..point + 1,
                "source={source:?}, point={point}"
            );
        }
    }
}

#[test]
fn replacement_across_adjacent_math_spans_keeps_the_unselected_formula_tail() {
    let source = "a $x+y$ and $q+r$ tail";
    let projection = VisualProjection::from_markdown(source);
    let visual = projection.text();
    assert_eq!(visual, "a x+y and q+r tail");
    for replacement in [" ", "新"] {
        let mut edited = visual.to_owned();
        edited.replace_range(byte_at(visual, 2)..byte_at(visual, 11), replacement);
        let cursor = 2 + replacement.chars().count();
        let update = projection
            .apply_edit(source, &edited, cursor..cursor)
            .unwrap();
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            edited,
            "replacement={replacement:?}, new_source={:?}",
            update.source
        );
    }
}
