use rupora::wysiwyg::VisualProjection;

fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

#[test]
fn short_edits_across_common_inline_styles_preserve_visible_text() {
    let sources = [
        "a *em* b",
        "*italic abc*",
        "**strong abc**",
        "**outer *inner* rest**",
        "*one* **two** *three*",
        "[link text](https://example.test) tail",
        "~~struck text~~",
        "`code text` tail",
        "prefix **strong** _em_ suffix",
        "**a** ***b*** ~~c~~ *d*",
    ];
    let mut failures = Vec::new();
    for source in sources {
        let projection = VisualProjection::from_markdown(source);
        let visual = projection.text();
        let length = visual.chars().count();
        for start in 0..length {
            for end in start + 1..=(start + 4).min(length) {
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
                    let reparsed = VisualProjection::from_markdown(&update.source);
                    if reparsed.text() != edited {
                        failures.push(format!(
                            "source={source:?} edit={start}..{end} replacement={replacement:?} expected={edited:?} actual={:?} new_source={:?}",
                            reparsed.text(), update.source
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
