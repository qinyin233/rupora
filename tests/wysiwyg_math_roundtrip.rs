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
                for replacement in ["", "新", "x"] {
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
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
