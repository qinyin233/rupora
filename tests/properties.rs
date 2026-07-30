use proptest::prelude::*;
use rupora::{
    editing::{MarkdownCommand, apply_markdown_command},
    markdown::{BlockIndex, analyze, render_html_fragment},
    merge,
    table::{self, Alignment, MarkdownTable},
};

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn paired_formatting_round_trips_arbitrary_unicode(
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
            MarkdownCommand::InlineCode,
        ] {
            let mut text = original.clone();
            let next = apply_markdown_command(&mut text, selection.clone(), command);
            apply_markdown_command(&mut text, next, command);
            prop_assert_eq!(&text, &original);
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
        prop_assert!(html.len() <= source.len().saturating_mul(128).saturating_add(4096));
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
