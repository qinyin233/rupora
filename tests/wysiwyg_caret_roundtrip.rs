use rupora::wysiwyg::VisualProjection;

fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

#[test]
fn short_content_edits_preserve_the_visible_caret() {
    let sources = [
        "a b",
        "a *em* b",
        "**strong abc**",
        "prefix **strong** _em_ suffix",
        "- **bold** tail",
        "> *quoted* tail",
        "# **heading** tail",
        "| a b | c d |\n| --- | --- |\n| e f | g h |",
        "| 中文🙂 | **bravo** |\n| --- | --- |\n| e f | g h |",
    ];
    let mut failures = Vec::new();
    let mut checked = 0;
    for source in sources {
        let projection = VisualProjection::from_markdown(source);
        let visual = projection.text();
        for run in projection.runs_for(visual) {
            if run.style.marker
                || visual[byte_at(visual, run.range.start)..byte_at(visual, run.range.end)]
                    .contains('\n')
            {
                continue;
            }
            for start in run.range.start..run.range.end {
                for end in start + 1..=(start + 3).min(run.range.end) {
                    for replacement in ["", " ", "新"] {
                        let mut edited = visual.to_owned();
                        edited.replace_range(
                            byte_at(visual, start)..byte_at(visual, end),
                            replacement,
                        );
                        if edited == visual {
                            continue;
                        }
                        let caret = start + replacement.chars().count();
                        let update = projection
                            .apply_edit(source, &edited, caret..caret)
                            .unwrap();
                        let active = VisualProjection::from_markdown_with_selection(
                            &update.source,
                            Some(update.selection.clone()),
                        );
                        checked += 1;
                        if active.text() != edited
                            || active.visual_char_range(&update.source, update.selection.clone())
                                != (caret..caret)
                        {
                            failures.push(format!(
                                "source={source:?} edit={start}..{end} replacement={replacement:?} caret={caret} visual={edited:?} actual={:?} actual_caret={:?} output={:?}",
                                active.text(),
                                active.visual_char_range(&update.source, update.selection),
                                update.source,
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "checked={checked}\n{}",
        failures.join("\n")
    );
}
