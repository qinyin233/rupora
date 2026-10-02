use rupora::{
    editing::{MarkdownCommand, apply_markdown_command},
    wysiwyg::VisualProjection,
};

#[test]
fn replacing_link_label_with_a_bracket_keeps_the_link() {
    let source = "[a](u)";
    let before = VisualProjection::from_markdown(source);
    let update = before.apply_edit(source, "[", 1..1).unwrap();
    let after = VisualProjection::from_markdown(&update.source);
    assert_eq!(after.text(), "[");
    assert!(
        after
            .runs_for(after.text())
            .iter()
            .all(|run| run.style.link),
        "{source:?} => {:?}",
        update.source
    );
    assert!(rupora::markdown::render_html_fragment(&update.source).contains("href=\"u\""));
}

#[test]
fn edits_after_link_creation_preserve_text_and_caret() {
    for prefix in ["前!", "前\\", "前\\\\!", "前\\!"] {
        let mut source = format!("{prefix}甲🙂尾");
        let start = prefix.chars().count();
        apply_markdown_command(&mut source, start..start + 2, MarkdownCommand::Link);
        let before = VisualProjection::from_markdown(&source);
        let chars: Vec<_> = before.text().chars().collect();
        for start in 0..=chars.len() {
            for end in start..=chars.len() {
                for replacement in ["新🙂", "", " "] {
                    if start == end && replacement.is_empty() {
                        continue;
                    }
                    let edited = chars[..start].iter().collect::<String>()
                        + replacement
                        + &chars[end..].iter().collect::<String>();
                    let caret = start + replacement.chars().count();
                    let update = before.apply_edit(&source, &edited, caret..caret).unwrap();
                    let after = VisualProjection::from_markdown(&update.source);
                    assert_eq!(
                        after.text(),
                        edited,
                        "{source:?} => {:?}; range={start}..{end}",
                        update.source
                    );
                    assert_eq!(
                        after.visual_char_range(&update.source, update.selection),
                        caret..caret,
                        "{:?}",
                        update.source
                    );
                }
            }
        }
    }
}

#[test]
fn edits_inside_created_link_preserve_its_destination_and_style() {
    for prefix in ["前!", "前\\", "前\\\\!", "前\\!"] {
        let mut source = format!("{prefix}甲🙂乙尾");
        let start = prefix.chars().count();
        let selected = apply_markdown_command(&mut source, start..start + 3, MarkdownCommand::Link);
        let before = VisualProjection::from_markdown(&source);
        let label = before.visual_char_range(&source, selected);
        let chars: Vec<_> = before.text().chars().collect();
        for start in label.clone() {
            for end in start..=label.end {
                // A caret at the label's opening boundary can intentionally
                // insert outside the link; select text or insert inside it.
                if start == label.start && start == end {
                    continue;
                }
                for replacement in ["新🙂", "", " ", "[", "]", "\\", "!"] {
                    if start == end && replacement.is_empty() {
                        continue;
                    }
                    let edited = chars[..start].iter().collect::<String>()
                        + replacement
                        + &chars[end..].iter().collect::<String>();
                    let caret = start + replacement.chars().count();
                    let update = before.apply_edit(&source, &edited, caret..caret).unwrap();
                    let after = VisualProjection::from_markdown(&update.source);
                    assert_eq!(after.text(), edited, "{source:?} => {:?}", update.source);
                    assert_eq!(
                        after.visual_char_range(&update.source, update.selection.clone()),
                        caret..caret,
                        "{:?}",
                        update.source
                    );
                    let expected =
                        label.start..label.end - (end - start) + replacement.chars().count();
                    for run in after.runs_for(after.text()) {
                        for i in run.range {
                            assert_eq!(
                                run.style.link,
                                expected.contains(&i),
                                "{source:?} => {:?}; edit={start}..{end} replacement={replacement:?} char={i}",
                                update.source
                            );
                        }
                    }
                    if !expected.is_empty() {
                        let destinations: Vec<_> = pulldown_cmark::Parser::new_ext(
                            &update.source,
                            rupora::markdown::parser_options(),
                        )
                        .filter_map(|event| match event {
                            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
                                dest_url,
                                ..
                            }) => Some(dest_url.into_string()),
                            _ => None,
                        })
                        .collect();
                        assert_eq!(destinations, ["https://"], "{:?}", update.source);
                    }
                }
            }
        }
    }
}

#[test]
fn link_label_punctuation_preserves_references_entities_styles_and_selections() {
    for source in [
        "前[a🙂b](u \"title\")尾",
        "前[a🙂b][id]尾\n\n[id]: u \"title\"",
        "前[a🙂b][]尾\n\n[a🙂b]: u \"title\"",
        "前[a🙂b]尾\n\n[a🙂b]: u \"title\"",
        "前[*a🙂b*](u \"title\")尾",
        "前[&fjlig;🙂b](u \"title\")尾",
        "前[a\\[🙂b](u \"title\")尾",
    ] {
        let before = VisualProjection::from_markdown(source);
        let chars: Vec<_> = before.text().chars().collect();
        let old_styles = styles(&before);
        let label = 1..chars.len() - 1;
        for start in label.clone() {
            for end in start..=label.end {
                if start == label.start && start == end {
                    continue;
                }
                for replacement in ["[", "]", "\\", "新[🙂]", "[x](v)"] {
                    let edited = chars[..start].iter().collect::<String>()
                        + replacement
                        + &chars[end..].iter().collect::<String>();
                    if edited == before.text() {
                        continue;
                    }
                    let caret = start + replacement.chars().count();
                    for selected in [caret..caret, start..caret] {
                        let update = before
                            .apply_edit(source, &edited, selected.clone())
                            .unwrap();
                        let after = VisualProjection::from_markdown(&update.source);
                        assert_eq!(after.text(), edited, "{source:?} => {:?}", update.source);
                        assert_eq!(
                            after.visual_char_range(&update.source, update.selection),
                            selected,
                            "{:?}",
                            update.source
                        );
                        let new_styles = styles(&after);
                        assert_eq!(
                            new_styles[..start],
                            old_styles[..start],
                            "{:?}",
                            update.source
                        );
                        assert_eq!(
                            new_styles[caret..],
                            old_styles[end..],
                            "{:?}",
                            update.source
                        );
                        assert!(new_styles[start..caret].iter().all(|style| style.link));
                        assert_eq!(
                            destinations(&update.source),
                            [("u".to_owned(), "title".to_owned())]
                        );

                        let mut continued = edited.clone();
                        let byte = continued.char_indices().nth(caret).unwrap().0;
                        continued.insert_str(byte, "新🙂");
                        let next = after
                            .apply_edit(&update.source, &continued, caret + 2..caret + 2)
                            .unwrap();
                        let next_projection = VisualProjection::from_markdown(&next.source);
                        assert_eq!(next_projection.text(), continued, "{:?}", next.source);
                        assert_eq!(
                            next_projection.visual_char_range(&next.source, next.selection),
                            caret + 2..caret + 2
                        );
                        assert_eq!(
                            destinations(&next.source),
                            [("u".to_owned(), "title".to_owned())]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn valid_markdown_and_revealed_link_source_keep_their_editing_behavior() {
    for replacement in ["*新🙂*", "`新🙂`", "![新🙂](image.png)"] {
        let source = "[a](u)";
        let before = VisualProjection::from_markdown(source);
        let caret = replacement.chars().count();
        let update = before
            .apply_edit(source, replacement, caret..caret)
            .unwrap();
        assert_eq!(update.source, format!("[{replacement}](u)"));
        assert_eq!(
            destinations(&update.source),
            [("u".to_owned(), String::new())]
        );
    }
    let source = "[a](u)";
    let revealed = VisualProjection::from_markdown_with_selection(source, Some(1..2));
    assert_eq!(revealed.text(), source);
    let update = revealed.apply_edit(source, "[[](u)", 2..2).unwrap();
    assert_eq!(update.source, "[[](u)");
}

#[test]
fn backslash_before_retained_inline_syntax_keeps_link_text_and_style() {
    for source in ["[a`b`](u)", "[a*b*](u)"] {
        let before = VisualProjection::from_markdown(source);
        let update = before.apply_edit(source, "\\b", 1..1).unwrap();
        let after = VisualProjection::from_markdown(&update.source);
        assert_eq!(after.text(), "\\b", "{:?}", update.source);
        assert_eq!(styles(&after)[1], styles(&before)[1]);
        assert_eq!(
            destinations(&update.source),
            [("u".to_owned(), String::new())]
        );
    }
}

#[test]
fn mixed_markdown_and_literal_brackets_keep_valid_inline_syntax() {
    for (typed, expected, code, image) in [
        ("`[` [", "[ [", true, false),
        ("![img](pic) [", "▧ img [", false, true),
    ] {
        let source = "[a](u)";
        let before = VisualProjection::from_markdown(source);
        let caret = typed.chars().count();
        let update = before.apply_edit(source, typed, caret..caret).unwrap();
        let after = VisualProjection::from_markdown(&update.source);
        assert_eq!(after.text(), expected, "{:?}", update.source);
        assert_eq!(styles(&after)[0].code, code);
        assert_eq!(
            destinations(&update.source),
            [("u".to_owned(), String::new())]
        );
        assert_eq!(
            rupora::markdown::render_html_fragment(&update.source).contains("<img"),
            image
        );
        let visual_caret = expected.chars().count();
        assert_eq!(
            after.visual_char_range(&update.source, update.selection),
            visual_caret..visual_caret
        );
    }
}

#[test]
fn an_empty_link_with_the_same_target_cannot_replace_the_visible_link() {
    let source = "[a](u)";
    let before = VisualProjection::from_markdown(source);
    let update = before.apply_edit(source, "](u)", 4..4).unwrap();
    let after = VisualProjection::from_markdown(&update.source);
    assert_eq!(after.text(), "](u)");
    assert!(
        styles(&after).iter().all(|style| style.link),
        "{:?}",
        update.source
    );
    assert_eq!(
        destinations(&update.source),
        [("u".to_owned(), String::new())]
    );
}

#[test]
fn mixed_math_and_reference_images_keep_their_context_and_literal_content() {
    for (source, typed, expected, image) in [
        ("[a](u)\n\n[p]: pic", "![img][p] [", "▧ img [", true),
        ("[a](u)", "$[$ [", "[ [", false),
    ] {
        let before = VisualProjection::from_markdown(source);
        let caret = typed.chars().count();
        let update = before.apply_edit(source, typed, caret..caret).unwrap();
        let after = VisualProjection::from_markdown(&update.source);
        assert_eq!(after.text(), expected, "{:?}", update.source);
        assert_eq!(
            destinations(&update.source),
            [("u".to_owned(), String::new())]
        );
        assert_eq!(
            rupora::markdown::render_html_fragment(&update.source).contains("src=\"pic\""),
            image
        );
        let visible_caret = expected.chars().count();
        assert_eq!(
            after.visual_char_range(&update.source, update.selection),
            visible_caret..visible_caret
        );
    }
}

#[test]
fn complete_markdown_escapes_survive_mixed_literal_bracket_input() {
    for (typed, expected) in [
        (r"\[ [", "[ ["),
        (r"\\ [", r"\ ["),
        (r"\* [", "* ["),
        (r"\] [", "] ["),
        ("\\[ 新🙂 [", "[ 新🙂 ["),
    ] {
        let source = "[a](u)";
        let before = VisualProjection::from_markdown(source);
        let caret = typed.chars().count();
        let update = before.apply_edit(source, typed, caret..caret).unwrap();
        let after = VisualProjection::from_markdown(&update.source);
        assert_eq!(after.text(), expected, "{typed:?} => {:?}", update.source);
        assert!(styles(&after).iter().all(|style| style.link));
        assert_eq!(
            destinations(&update.source),
            [("u".to_owned(), String::new())]
        );
        let visible_caret = expected.chars().count();
        assert_eq!(
            after.visual_char_range(&update.source, update.selection),
            visible_caret..visible_caret
        );
    }
}

#[test]
fn link_label_edits_preserve_unselected_literal_backslashes() {
    for count in 1..=4 {
        let source = format!("[{}a](u)", "\\".repeat(count));
        let before = VisualProjection::from_markdown(&source);
        let prefix = "\\".repeat(count.div_ceil(2));
        assert_eq!(before.text(), format!("{prefix}a"));
        for (typed, rendered) in [
            ("[", "["),
            ("]", "]"),
            ("\\", "\\"),
            (r"\[ [", "[ ["),
            (r"\\ [", r"\ ["),
            (r"\* [", "* ["),
        ] {
            for trailing in ["", "a"] {
                let edited = format!("{prefix}{typed}{trailing}");
                let expected = format!("{prefix}{rendered}{trailing}");
                let caret = prefix.chars().count() + typed.chars().count();
                let update = before.apply_edit(&source, &edited, caret..caret).unwrap();
                let after = VisualProjection::from_markdown(&update.source);
                assert_eq!(
                    after.text(),
                    expected,
                    "{source:?} + {typed:?} => {:?}",
                    update.source
                );
                assert!(styles(&after).iter().all(|style| style.link));
                assert_eq!(
                    destinations(&update.source),
                    [("u".to_owned(), String::new())]
                );
                let visible_caret = prefix.chars().count() + rendered.chars().count();
                assert_eq!(
                    after.visual_char_range(&update.source, update.selection),
                    visible_caret..visible_caret
                );
            }
        }
    }
}

fn styles(projection: &VisualProjection) -> Vec<rupora::wysiwyg::VisualStyle> {
    projection
        .runs_for(projection.text())
        .into_iter()
        .flat_map(|run| std::iter::repeat_n(run.style, run.range.len()))
        .collect()
}

fn destinations(source: &str) -> Vec<(String, String)> {
    pulldown_cmark::Parser::new_ext(source, rupora::markdown::parser_options())
        .filter_map(|event| match event {
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
                dest_url, title, ..
            }) => Some((dest_url.into_string(), title.into_string())),
            _ => None,
        })
        .collect()
}
