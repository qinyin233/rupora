use rupora::{
    editing::{MarkdownCommand, apply_markdown_command},
    markdown::render_html_fragment,
    wysiwyg::VisualProjection,
};

#[test]
fn fallback_emphasis_survives_a_visual_paragraph_split() {
    for source in [
        "<strong>甲🙂</strong>尾",
        "<strong><em>甲🙂</em></strong>尾",
    ] {
        let before = VisualProjection::from_markdown(source);
        let mut update = before.apply_edit(source, "甲\n🙂尾", 2..2).unwrap();
        update.selection =
            rupora::wysiwyg::complete_visual_enter(&mut update.source, update.selection, false);
        let after = VisualProjection::from_markdown(&update.source);
        assert_eq!(after.text(), "甲\n\n🙂尾", "{:?}", update.source);
        let styles: Vec<_> = after
            .runs_for(after.text())
            .into_iter()
            .flat_map(|run| std::iter::repeat_n(run.style, run.range.len()))
            .collect();
        assert!(styles[0].strong && styles[3].strong, "{:?}", update.source);
        assert!(!styles[4].strong);
        if source.contains("<em>") {
            assert!(styles[0].emphasis && styles[3].emphasis);
        }
        assert_eq!(
            after.visual_char_range(&update.source, update.selection),
            3..3
        );
        let joined = after.apply_edit(&update.source, "甲🙂尾", 1..1).unwrap();
        let rejoined = VisualProjection::from_markdown(&joined.source);
        assert_eq!(rejoined.text(), "甲🙂尾", "{:?}", joined.source);
        assert_eq!(
            rejoined.visual_char_range(&joined.source, joined.selection),
            1..1
        );
        for run in rejoined.runs_for(rejoined.text()) {
            for i in run.range {
                assert_eq!(run.style.strong, i < 2, "{:?}", joined.source);
                assert_eq!(run.style.emphasis, i < 2 && source.contains("<em>"));
            }
        }
    }
}

#[test]
fn entering_at_html_format_edges_keeps_existing_styles() {
    for (edited, caret, expected) in [("\n甲🙂尾", 1, "\n\n甲🙂尾"), ("甲🙂\n尾", 3, "甲🙂\n\n尾")]
    {
        let source = "<strong><em>甲🙂</em></strong>尾";
        let before = VisualProjection::from_markdown(source);
        let mut update = before.apply_edit(source, edited, caret..caret).unwrap();
        assert_eq!(
            update.source.chars().nth(update.selection.end - 1),
            Some('\n'),
            "Enter caret must follow its inserted newline: {:?} at {:?}",
            update.source,
            update.selection
        );
        update.selection =
            rupora::wysiwyg::complete_visual_enter(&mut update.source, update.selection, false);
        let after = VisualProjection::from_markdown(&update.source);
        assert_eq!(after.text(), expected, "{:?}", update.source);
        let styles: Vec<_> = after
            .runs_for(after.text())
            .into_iter()
            .flat_map(|run| std::iter::repeat_n(run.style, run.range.len()))
            .collect();
        for (i, ch) in after.text().chars().enumerate() {
            if ch == '甲' || ch == '🙂' {
                assert!(
                    styles[i].strong && styles[i].emphasis,
                    "{:?}",
                    update.source
                );
            }
            if ch == '尾' {
                assert!(!styles[i].strong && !styles[i].emphasis);
            }
        }
    }
}

#[test]
fn fallback_formats_compose_and_toggle_without_changing_neighbors() {
    for (original, selected) in [
        ("!a", 0..1),
        ("a!", 1..2),
        ("a!b", 1..2),
        ("甲🙂尾\n\n乙", 0..2),
    ] {
        for (first, second) in [
            (MarkdownCommand::Bold, MarkdownCommand::Italic),
            (MarkdownCommand::Italic, MarkdownCommand::Bold),
        ] {
            let mut source = original.to_owned();
            let one = apply_markdown_command(&mut source, selected.clone(), first);
            let first_source = source.clone();
            let two = apply_markdown_command(&mut source, one.clone(), second);
            let projection = VisualProjection::from_markdown(&source);
            assert_eq!(projection.text(), original, "{source:?}");
            let styles: Vec<_> = projection
                .runs_for(projection.text())
                .into_iter()
                .flat_map(|run| std::iter::repeat_n(run.style, run.range.len()))
                .collect();
            for (i, style) in styles.iter().enumerate() {
                assert_eq!(style.strong, selected.contains(&i), "{source:?} index={i}");
                assert_eq!(
                    style.emphasis,
                    selected.contains(&i),
                    "{source:?} index={i}"
                );
            }
            let composed = source.clone();
            let first_off = apply_markdown_command(&mut source, two.clone(), first);
            let restored_both = apply_markdown_command(&mut source, first_off, first);
            assert_eq!(source, composed);
            assert_eq!(restored_both, two);
            let back = apply_markdown_command(&mut source, two, second);
            assert_eq!(source, first_source);
            assert_eq!(back, one);
            let restored = apply_markdown_command(&mut source, back, first);
            assert_eq!(source, original);
            assert_eq!(restored, selected);
        }
    }
}

#[test]
fn emphasis_after_a_literal_backslash_preserves_visible_neighbors() {
    for (original, selected) in [
        ("x", 0..1),
        ("\\\\x", 2..3),
        ("\\x", 1..2),
        ("甲\\x尾", 2..3),
        ("\\\\\\x", 3..4),
        ("\\\\\\\\x", 4..5),
        ("甲\\🙂尾", 2..3),
        ("\\甲\\尾", 1..3),
        ("前\\甲🙂\\尾", 2..5),
    ] {
        for command in [MarkdownCommand::Bold, MarkdownCommand::Italic] {
            let mut source = original.to_owned();
            let next = apply_markdown_command(&mut source, selected.clone(), command);
            let projection = VisualProjection::from_markdown(&source);
            let before = VisualProjection::from_markdown(original);
            let expected = before.visual_char_range(original, selected.clone());
            assert_eq!(projection.text(), before.text(), "{source:?}");
            assert_eq!(
                projection.visual_char_range(&source, next.clone()),
                expected
            );
            for run in projection.runs_for(projection.text()) {
                for i in run.range {
                    assert_eq!(
                        run.style.strong,
                        expected.contains(&i) && command == MarkdownCommand::Bold,
                        "{original:?} => {source:?}"
                    );
                    assert_eq!(
                        run.style.emphasis,
                        expected.contains(&i) && command == MarkdownCommand::Italic,
                        "{original:?} => {source:?}"
                    );
                }
            }
            let other = if command == MarkdownCommand::Bold {
                MarkdownCommand::Italic
            } else {
                MarkdownCommand::Bold
            };
            let mut next = next;
            for (toggle, strong, emphasis) in [
                (other, true, true),
                (
                    command,
                    other == MarkdownCommand::Bold,
                    other == MarkdownCommand::Italic,
                ),
                (other, false, false),
                (
                    command,
                    command == MarkdownCommand::Bold,
                    command == MarkdownCommand::Italic,
                ),
                (command, false, false),
            ] {
                next = apply_markdown_command(&mut source, next, toggle);
                let projection = VisualProjection::from_markdown(&source);
                assert_eq!(projection.text(), before.text(), "{source:?}");
                assert_eq!(
                    projection.visual_char_range(&source, next.clone()),
                    expected
                );
                for run in projection.runs_for(projection.text()) {
                    for i in run.range {
                        assert_eq!(
                            run.style.strong,
                            expected.contains(&i) && strong,
                            "{source:?}"
                        );
                        assert_eq!(
                            run.style.emphasis,
                            expected.contains(&i) && emphasis,
                            "{source:?}"
                        );
                    }
                }
            }
            assert_eq!(
                render_html_fragment(&source),
                render_html_fragment(original)
            );
        }
    }
}

#[test]
fn preceding_backslash_repair_does_not_rewrite_code_or_partial_escapes() {
    for (original, selected) in [("`\\x`", 2..3), ("\\!", 1..2), ("\\*", 1..2)] {
        for command in [MarkdownCommand::Bold, MarkdownCommand::Italic] {
            let mut source = original.to_owned();
            let next = apply_markdown_command(&mut source, selected.clone(), command);
            assert_eq!(source, original);
            assert_eq!(next, selected);
        }
    }
}

#[test]
fn literal_backslashes_can_be_formatted_beside_letters() {
    for (original, selected) in [
        ("\\a", 0..1),
        ("甲\\尾", 1..2),
        ("甲\\尾", 0..2),
        ("\\\\\\a", 0..3),
    ] {
        for command in [MarkdownCommand::Bold, MarkdownCommand::Italic] {
            let mut source = original.to_owned();
            let next = apply_markdown_command(&mut source, selected.clone(), command);
            let before = VisualProjection::from_markdown(original);
            let projection = VisualProjection::from_markdown(&source);
            let expected = before.visual_char_range(original, selected.clone());
            assert_eq!(projection.text(), before.text(), "{source:?}");
            assert_eq!(
                projection.visual_char_range(&source, next.clone()),
                expected
            );
            for run in projection.runs_for(projection.text()) {
                for i in run.range {
                    assert_eq!(
                        run.style.strong,
                        expected.contains(&i) && command == MarkdownCommand::Bold,
                        "{source:?}"
                    );
                    assert_eq!(
                        run.style.emphasis,
                        expected.contains(&i) && command == MarkdownCommand::Italic,
                        "{source:?}"
                    );
                }
            }
            apply_markdown_command(&mut source, next, command);
            let restored = VisualProjection::from_markdown(&source);
            assert_eq!(restored.text(), before.text(), "{source:?}");
            assert!(
                restored
                    .runs_for(restored.text())
                    .iter()
                    .all(|r| !r.style.strong && !r.style.emphasis)
            );
        }
    }
}

#[test]
fn mixed_emphasis_encodings_toggle_each_style_independently() {
    for (original, selected) in [
        ("<strong>*甲🙂*</strong>尾", 9..11),
        ("<strong>*x*</strong> tail", 9..10),
        ("**<em>x</em>** tail", 6..7),
    ] {
        for command in [MarkdownCommand::Bold, MarkdownCommand::Italic] {
            let mut source = original.to_owned();
            let next = apply_markdown_command(&mut source, selected.clone(), command);
            let projection = VisualProjection::from_markdown(&source);
            let before = VisualProjection::from_markdown(original);
            assert_eq!(
                projection.text(),
                before.text(),
                "{original:?} => {source:?}"
            );
            let selected_count = selected.len();
            for run in projection.runs_for(projection.text()) {
                for i in run.range {
                    assert_eq!(
                        run.style.strong,
                        i < selected_count && command != MarkdownCommand::Bold,
                        "{source:?}"
                    );
                    assert_eq!(
                        run.style.emphasis,
                        i < selected_count && command != MarkdownCommand::Italic,
                        "{source:?}"
                    );
                }
            }
            assert_eq!(
                projection.visual_char_range(&source, next),
                0..selected_count
            );
        }
    }
}

#[test]
fn fallback_projection_keeps_unicode_edits_and_caret_mapped() {
    for source in [
        "<strong>甲🙂</strong>尾\n\n乙",
        "<em>甲🙂</em>尾\n\n乙",
        "<strong>*甲🙂*</strong>尾\n\n乙",
    ] {
        let before = VisualProjection::from_markdown(source);
        assert_eq!(before.text(), "甲🙂尾\n\n乙");
        for range in [0..0, 1..1, 2..2, 1..2, 0..2] {
            for text in ["", "新🙂", " "] {
                if range.is_empty() && text.is_empty() {
                    continue;
                }
                let chars: Vec<_> = before.text().chars().collect();
                let edited = chars[..range.start].iter().collect::<String>()
                    + text
                    + &chars[range.end..].iter().collect::<String>();
                let caret = range.start + text.chars().count();
                let result = before.apply_edit(source, &edited, caret..caret).unwrap();
                let after = VisualProjection::from_markdown(&result.source);
                assert_eq!(after.text(), edited, "{source:?} => {:?}", result.source);
                assert_eq!(
                    after.visual_char_range(&result.source, result.selection),
                    caret..caret
                );
                let styles: Vec<_> = after
                    .runs_for(after.text())
                    .into_iter()
                    .flat_map(|run| std::iter::repeat_n(run.style, run.range.len()))
                    .collect();
                let original_styles: Vec<_> = before
                    .runs_for(before.text())
                    .into_iter()
                    .flat_map(|run| std::iter::repeat_n(run.style, run.range.len()))
                    .collect();
                for (old, new) in (0..range.start)
                    .map(|i| (i, i))
                    .chain((range.end..chars.len()).map(|i| (i, caret + i - range.end)))
                {
                    assert_eq!(
                        styles[new].strong, original_styles[old].strong,
                        "{source:?} => {:?}, unchanged char {old}",
                        result.source
                    );
                    assert_eq!(
                        styles[new].emphasis, original_styles[old].emphasis,
                        "{source:?} => {:?}, unchanged char {old}",
                        result.source
                    );
                }
            }
        }
        assert!(render_html_fragment(source).contains("<p>乙</p>"));
    }
}

#[test]
fn inline_html_styles_do_not_cross_code_blocks_or_malformed_tags() {
    for source in [
        "`<strong>x</strong>`",
        "<strong>x\n\ny</strong>",
        "<strong>*x</strong>*",
        "<strong><em>x</strong></em>",
        "<strong>x",
        "<strong class=x>x</strong>",
    ] {
        let projection = VisualProjection::from_markdown(source);
        assert!(
            projection
                .runs_for(projection.text())
                .iter()
                .all(|run| !run.style.strong),
            "{source:?}"
        );
    }
    let source = "<strong>*x*</strong> tail";
    let projection = VisualProjection::from_markdown(source);
    assert_eq!(projection.text(), "x tail");
    let runs = projection.runs_for(projection.text());
    assert!(runs[0].style.strong && runs[0].style.emphasis);
    assert!(!runs.last().unwrap().style.strong);
    let with_comment = VisualProjection::from_markdown("<strong><!-- comment -->x</strong>");
    assert!(with_comment.runs_for(with_comment.text())[0].style.strong);
}

#[test]
fn deep_html_fallbacks_keep_the_markdown_event_stack_bounded() {
    let depth = 1024;
    let source = "<strong>".repeat(depth) + &"*x* ".repeat(depth) + &"</strong>".repeat(depth);
    let projection = VisualProjection::from_markdown(&source);
    assert_eq!(projection.text().matches('x').count(), depth);
    assert!(
        projection
            .runs_for(projection.text())
            .iter()
            .all(|run| run.style.strong)
    );

    // A partial selection does not match any enclosing wrapper's full body.
    // Previously each candidate walked every descendant and rescanned all
    // spans at every step, giving this command cubic work in nesting depth.
    let mut source = "<strong>".repeat(depth) + "xy" + &"</strong>".repeat(depth);
    let start = "<strong>".len() * depth;
    let selected = apply_markdown_command(&mut source, start..start + 1, MarkdownCommand::Italic);
    let projection = VisualProjection::from_markdown(&source);
    assert_eq!(projection.text(), "xy");
    assert_eq!(projection.visual_char_range(&source, selected), 0..1);
    let runs = projection.runs_for(projection.text());
    assert!(runs[0].style.strong && runs[0].style.emphasis);
    assert!(runs.last().unwrap().style.strong);
    assert!(!runs.last().unwrap().style.emphasis);
}
