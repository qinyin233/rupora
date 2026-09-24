use proptest::prelude::*;
use rupora::{
    editing::{MarkdownCommand, apply_markdown_command},
    markdown::{BlockIndex, MAX_GENERATED_DOCUMENT_BYTES, analyze, render_html_fragment},
    merge,
    table::{self, Alignment, MarkdownTable},
};

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct("tests/properties.proptest-regressions")
        )),
        ..ProptestConfig::default()
    })]

    #[test]
    fn fixed_marker_formatting_round_trips_arbitrary_unicode(
        characters in prop::collection::vec(any::<char>(), 0..200),
        left in any::<usize>(),
        right in any::<usize>(),
    ) {
        let original = characters.into_iter().collect::<String>();
        let length = original.chars().count();
        let left = left % (length + 1);
        let right = right % (length + 1);
        let selection = left.min(right)..left.max(right);

        for command in [
            MarkdownCommand::Bold,
            MarkdownCommand::Italic,
            MarkdownCommand::Strikethrough,
        ] {
            let mut text = original.clone();
            let next = apply_markdown_command(&mut text, selection.clone(), command);
            apply_markdown_command(&mut text, next, command);
            prop_assert_eq!(&text, &original);
        }
    }

    #[test]
    fn inline_code_additions_round_trip_and_removals_preserve_content(
        characters in prop::collection::vec(prop_oneof![
            4 => any::<char>(),
            2 => Just('`'),
            1 => Just(' '),
            1 => Just('\n'),
        ], 0..200),
        left in any::<usize>(),
        right in any::<usize>(),
    ) {
        let original = characters.into_iter().collect::<String>();
        let length = original.chars().count();
        let left = left % (length + 1);
        let right = right % (length + 1);
        let selection = left.min(right)..left.max(right);
        let mut text = original.clone();
        let next = apply_markdown_command(&mut text, selection.clone(), MarkdownCommand::InlineCode);

        if text.len() > original.len() {
            apply_markdown_command(&mut text, next, MarkdownCommand::InlineCode);
            prop_assert_eq!(text, original);
        } else {
            // Removing existing code preserves its selected content. Reapplying
            // uses a minimal fence, so nonminimal original fences need not return
            // byte for byte: ``foo`` -> foo -> `foo` is a valid toggle sequence.
            let original_content = original.chars()
                .skip(selection.start).take(selection.len()).collect::<String>();
            let remaining_content = text.chars()
                .skip(next.start).take(next.len()).collect::<String>();
            prop_assert_eq!(remaining_content, original_content);
        }
    }

    #[test]
    fn markdown_analysis_and_block_index_accept_arbitrary_utf8(
        characters in prop::collection::vec(any::<char>(), 0..2_000),
    ) {
        let source = characters.into_iter().collect::<String>();
        let analysis = analyze(&source);
        prop_assert!(analysis.lines >= 1);
        prop_assert!(analysis.characters <= source.chars().count());

        let mut index = BlockIndex::new(&source);
        let updated = format!("prefix\n\n{source}\n\nsuffix");
        index.update(&updated);
        prop_assert!(!index.blocks().is_empty());
        let html = render_html_fragment(&source);
        // Embedded SVG glyph outlines are governed by the generated-document
        // budget, not a fixed expansion ratio relative to a short formula.
        let generated_budget = if html.contains("<svg") { MAX_GENERATED_DOCUMENT_BYTES } else { 0 };
        prop_assert!(html.len() <= source.len().saturating_mul(128).saturating_add(4096).saturating_add(generated_budget));
    }

    #[test]
    fn table_round_trip_preserves_rectangular_cells(
        headers in prop::collection::vec("[a-zA-Z0-9\u{4e00}-\u{9fa5}]{0,16}", 1..8),
        flat_cells in prop::collection::vec("[a-zA-Z0-9\u{4e00}-\u{9fa5}]{0,16}", 0..48),
    ) {
        let columns = headers.len();
        let rows = flat_cells
            .chunks(columns)
            .map(|chunk| {
                let mut row = chunk.to_vec();
                row.resize(columns, String::new());
                row
            })
            .collect::<Vec<_>>();
        let table = MarkdownTable {
            range: 0..0,
            headers: headers.clone(),
            alignments: vec![Alignment::Center; columns],
            rows,
        };
        let markdown = table.to_markdown();
        let reparsed = table::find_table(&markdown, 0).expect("serialized table");
        prop_assert_eq!(reparsed.headers, headers);
        prop_assert!(reparsed.rows.iter().all(|row| row.len() == columns));
    }

    #[test]
    fn three_way_merge_always_keeps_non_conflicting_single_side_changes(
        base in "([a-z]{0,20}\n){0,20}",
        local_suffix in "[a-z]{0,20}",
        external_suffix in "[a-z]{0,20}",
    ) {
        let local = format!("{base}{local_suffix}");
        let unchanged_external = merge::three_way_merge(&base, &local, &base);
        prop_assert_eq!(unchanged_external.content, local);
        prop_assert_eq!(unchanged_external.conflicts, 0);

        let external = format!("{base}{external_suffix}");
        let unchanged_local = merge::three_way_merge(&base, &base, &external);
        prop_assert_eq!(unchanged_local.content, external);
        prop_assert_eq!(unchanged_local.conflicts, 0);
    }
}

#[test]
fn source_audit_math_output_budget_accounts_for_embedded_glyphs() {
    // Minimized by the full-suite property run on 2026-09-10. Its valid SVG
    // exceeded the old source-length ratio when the installed CJK font was used.
    let source = " ¡ $¡¡0 A00𠀀!!$ ";
    let html = render_html_fragment(source);
    assert!(html.contains("class=\"math-inline\""));
    assert!(html.contains("<svg"));
    assert!(html.len() <= source.len() * 128 + 4096 + MAX_GENERATED_DOCUMENT_BYTES);
}
