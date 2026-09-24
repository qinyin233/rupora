use rupora::wysiwyg::VisualProjection;

fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

#[test]
fn short_edits_inside_table_cells_preserve_visible_text() {
    let sources = [
        "| alpha | **bravo** |\n| --- | --- |\n| 中文🙂 | delta |",
        "| a b | c d |\n| --- | --- |\n| e f | g h |",
    ];
    let mut failures = Vec::new();
    for source in sources {
        let projection = VisualProjection::from_markdown(source);
        let visual = projection.text();
        for run in projection.runs_for(visual) {
            // Table separators and row boundaries are structural. Restrict
            // this text invariant to one editable cell content run at a time.
            if !run.style.table
                || run.style.marker
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
                        let cursor = start + replacement.chars().count();
                        let update = projection
                            .apply_edit(source, &edited, cursor..cursor)
                            .unwrap();
                        let actual = VisualProjection::from_markdown(&update.source);
                        if actual.text() != edited {
                            failures.push(format!("source={source:?} edit={start}..{end} replacement={replacement:?} expected={edited:?} actual={:?} new_source={:?}", actual.text(), update.source));
                        }
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
