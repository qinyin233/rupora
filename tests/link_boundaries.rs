use rupora::{
    editing::{MarkdownCommand, apply_markdown_command},
    markdown::{parser_options, render_html_fragment},
    wysiwyg::VisualProjection,
};

#[test]
fn link_command_preserves_literal_prefix_and_creates_a_link() {
    check_command(&[("x", 0..1), ("!x", 1..2)]);
}

#[test]
fn link_command_preserves_literal_backslash_prefix() {
    check_command(&[("x", 0..1), ("\\\\x", 2..3), ("\\x", 1..2)]);
}

#[test]
fn link_command_maps_unicode_and_preserves_existing_label_styles() {
    for prefix in [
        "前!",
        "前\\",
        "前\\\\",
        "前\\\\\\",
        "前\\!",
        "前\\\\!",
        "前!!",
    ] {
        for label in ["甲🙂", "甲*文*🙂", "甲<strong>文🙂</strong>"] {
            let source = format!("{prefix}{label}尾");
            let start = prefix.chars().count();
            check_command(&[(&source, start..start + label.chars().count())]);
        }
    }
}

fn check_command(cases: &[(&str, std::ops::Range<usize>)]) {
    for (original, selected) in cases {
        let before = VisualProjection::from_markdown(original);
        let expected = before.visual_char_range(original, selected.clone());
        let mut source = (*original).to_owned();
        let selection =
            apply_markdown_command(&mut source, selected.clone(), MarkdownCommand::Link);
        let links = pulldown_cmark::Parser::new_ext(&source, parser_options())
            .filter(|event| {
                matches!(
                    event,
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { .. })
                )
            })
            .count();
        assert_eq!(
            links,
            1,
            "{original:?} => {source:?}: {}",
            render_html_fragment(&source)
        );
        let after = VisualProjection::from_markdown(&source);
        assert_eq!(after.text(), before.text(), "{original:?} => {source:?}");
        assert_eq!(after.visual_char_range(&source, selection), expected);
        let old_styles: Vec<_> = before
            .runs_for(before.text())
            .into_iter()
            .flat_map(|run| std::iter::repeat_n(run.style, run.range.len()))
            .collect();
        for run in after.runs_for(after.text()) {
            for i in run.range {
                assert_eq!(run.style.link, expected.contains(&i), "{source:?} char={i}");
                assert_eq!(run.style.strong, old_styles[i].strong, "{source:?}");
                assert_eq!(run.style.emphasis, old_styles[i].emphasis, "{source:?}");
            }
        }
    }
}

#[test]
fn resource_images_and_empty_links_keep_their_intended_kind() {
    for prefix in ["!", "\\", "\\\\", "\\\\\\", "\\!", "\\\\!"] {
        let mut source = prefix.to_owned();
        let cursor = source.chars().count();
        let selected = apply_markdown_command(&mut source, cursor..cursor, MarkdownCommand::Link);
        assert_eq!(source.chars().nth(selected.start - 1), Some('['));
        assert_eq!(source.chars().nth(selected.start), Some(']'));
        let events: Vec<_> = pulldown_cmark::Parser::new_ext(&source, parser_options()).collect();
        assert!(
            events.iter().any(|e| matches!(
                e,
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { .. })
            )),
            "{source:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(
                e,
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. })
            )),
            "{source:?}"
        );

        let mut source = prefix.to_owned();
        let cursor = rupora::editing::insert_resource_link(
            &mut source,
            cursor..cursor,
            "甲🙂",
            "image.png",
            true,
        );
        assert_eq!(cursor, source.chars().count()..source.chars().count());
        let events: Vec<_> = pulldown_cmark::Parser::new_ext(&source, parser_options()).collect();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(
                    e,
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. })
                ))
                .count(),
            1,
            "{source:?}"
        );
        let visible_prefix: String = events
            .iter()
            .take_while(|e| {
                !matches!(
                    e,
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. })
                )
            })
            .filter_map(|e| {
                if let pulldown_cmark::Event::Text(t) = e {
                    Some(t.as_ref())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            visible_prefix,
            VisualProjection::from_markdown(prefix).text(),
            "{source:?}"
        );
    }
}

#[test]
fn literal_code_and_html_source_do_not_gain_prefix_escapes() {
    for (original, selection, expected) in [
        ("`!x`", 2..3, "`![x](https://)`"),
        ("`\\x`", 2..3, "`\\[x](https://)`"),
        ("```\n!x\n```", 5..6, "```\n![x](https://)\n```"),
        ("    !x", 5..6, "    ![x](https://)"),
        ("<a title='!x'>", 11..12, "<a title='![x](https://)'>"),
    ] {
        let mut source = original.to_owned();
        apply_markdown_command(&mut source, selection, MarkdownCommand::Link);
        assert_eq!(source, expected);
    }
}

#[test]
fn raw_partial_escape_keeps_its_existing_source_edit_behavior() {
    let mut source = r"\!".to_owned();
    let selection = apply_markdown_command(&mut source, 1..2, MarkdownCommand::Link);
    assert_eq!(source, r"\[!](https://)");
    assert_eq!(selection, 2..3);
}

#[test]
fn link_at_literal_block_eof_does_not_add_an_escape() {
    for original in ["```\n\\", "    \\", "<!-- \\"] {
        let mut source = original.to_owned();
        let end = source.chars().count();
        apply_markdown_command(&mut source, end..end, MarkdownCommand::Link);
        assert_eq!(source, format!("{original}[](https://)"));
    }
}

#[test]
fn pasted_url_preserves_literal_prefix() {
    for prefix in ["!", "\\", "前!", "前\\", "前\\\\", "前\\!", "前\\\\!"] {
        let original = format!("{prefix}甲🙂尾");
        let before = VisualProjection::from_markdown(&original);
        let start = prefix.chars().count();
        let expected = before.visual_char_range(&original, start..start + 2).end;
        let mut source = original.clone();
        let cursor = rupora::editing::paste_url_as_markdown_link(
            &mut source,
            start..start + 2,
            "https://example.com",
        )
        .unwrap();
        let after = VisualProjection::from_markdown(&source);
        assert_eq!(after.text(), before.text(), "{source:?}");
        assert_eq!(after.visual_char_range(&source, cursor), expected..expected);
        assert!(
            render_html_fragment(&source).contains("<a href="),
            "{source:?}"
        );
    }
}

#[test]
fn inserted_resource_link_preserves_literal_prefix() {
    for prefix in ["!", "\\", "前!", "前\\", "前\\\\", "前\\!", "前\\\\!"] {
        let original = format!("{prefix}甲🙂尾");
        let before = VisualProjection::from_markdown(&original);
        let start = prefix.chars().count();
        let expected = before.visual_char_range(&original, start..start + 2).end;
        let mut source = original.clone();
        let cursor = rupora::editing::insert_resource_link(
            &mut source,
            start..start + 2,
            "甲🙂",
            "notes.md",
            false,
        );
        let after = VisualProjection::from_markdown(&source);
        assert_eq!(after.text(), before.text(), "{source:?}");
        assert_eq!(after.visual_char_range(&source, cursor), expected..expected);
        assert!(
            render_html_fragment(&source).contains("<a href="),
            "{source:?}"
        );
    }
}
