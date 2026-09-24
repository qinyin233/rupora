#[path = "support/wysiwyg_projection.rs"]
mod projection_fuzz;

fn number(output: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        output.push(byte | if value != 0 { 0x80 } else { 0 });
        if value == 0 {
            break;
        }
    }
}

fn trace(source: &str, edits: &[(u8, u32, u32, &str)]) -> Vec<u8> {
    let mut output = Vec::new();
    number(&mut output, source.len() as u32);
    output.extend_from_slice(source.as_bytes());
    for (flags, first, second, replacement) in edits {
        output.push(*flags);
        number(&mut output, *first);
        number(&mut output, *second);
        number(&mut output, replacement.len() as u32);
        output.extend_from_slice(replacement.as_bytes());
    }
    output
}

#[test]
fn edits_unicode_after_the_old_source_and_offset_limits() {
    let prefix = "a".repeat(1024);
    let input = trace(&format!("{prefix}中文🙂尾"), &[(0, 1024, 1027, "新🙂")]);
    let (source, selection, steps) = projection_fuzz::run(&input).unwrap();
    assert_eq!(source, format!("{prefix}新🙂尾"));
    assert_eq!(selection, 1026..1026);
    assert_eq!(steps, 1);
}

#[test]
fn follows_multiple_edits_through_updated_unicode_source() {
    let input = trace("中文🙂尾", &[(0, 1, 3, "新"), (1, 2, 2, "🙂")]);
    let (source, selection, steps) = projection_fuzz::run(&input).unwrap();
    assert_eq!(source, "中新🙂尾");
    assert_eq!(selection, 3..3);
    assert_eq!(steps, 2);
}

#[test]
fn projection_trace_preserves_noop_and_selected_replacement_semantics() {
    let input = trace(
        "中文🙂",
        &[(0, 1, 2, "文"), (2, 1, 3, "新🙂"), (1, 3, 3, "尾")],
    );
    let (source, selection, steps) = projection_fuzz::run(&input).unwrap();
    assert_eq!(source, "中新🙂尾");
    assert_eq!(selection, 4..4);
    assert_eq!(steps, 3);
}

#[test]
fn projection_trace_checks_formatted_media_and_revealed_syntax() {
    for source in [
        "**前**![*中*](x)**后**",
        "[![**甲**](a)](outer)*乙*",
        "&NotEqualTilde; 中文🙂",
        "[文字](https://example.com/目标)",
        "- [ ] 中文\n\n```rust\nlet a = 1;\n```\n\n尾",
        "| 甲 | 乙 |\n| --- | --- |\n| 中 | 🙂 |",
        "+ \r\n  ",
    ] {
        for flags in 0..4 {
            let input = trace(
                source,
                &[(flags, 0, 1, "新"), (flags, 1, 3, "🙂"), (flags, 0, 0, "")],
            );
            let (_, _, steps) = projection_fuzz::run(&input).unwrap();
            assert_eq!(steps, 3, "source={source:?} flags={flags}");
        }
    }
}

#[test]
fn projection_trace_stops_at_its_step_budget() {
    let input = trace("", &[(0, 0, 0, "x"); 64]);
    let (source, selection, steps) = projection_fuzz::run(&input).unwrap();
    assert_eq!(steps, 32);
    assert_eq!(source, "x".repeat(32));
    assert_eq!(selection, 1..1);
}

#[test]
fn projection_trace_stops_before_reparsing_a_document_over_budget() {
    let initial = "a".repeat(64 * 1024);
    let replacement = "b".repeat(4 * 1024);
    let input = trace(&initial, &[(0, 0, 0, &replacement), (0, 0, 0, "ignored")]);
    let (source, selection, steps) = projection_fuzz::run(&input).unwrap();
    assert_eq!(steps, 1);
    assert_eq!(source, format!("{replacement}{initial}"));
    assert_eq!(selection, 4096..4096);
}

#[test]
fn projection_trace_handles_truncation_invalid_utf8_and_overflow_fields() {
    let input = trace("中文🙂", &[(0, 1, 2, "新"), (3, 1, 3, "🙂")]);
    for end in 0..=input.len() {
        let _ = projection_fuzz::run(&input[..end]);
    }
    assert!(projection_fuzz::run(&[1, 0xff]).is_none());
    assert!(projection_fuzz::run(&[0xff; 5]).is_none());
    assert!(projection_fuzz::run(&[0; 256 * 1024 + 1]).is_none());

    let mut invalid_edit = trace("原文", &[(0, 0, 1, "新")]);
    invalid_edit.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0x10]);
    let (source, _, steps) = projection_fuzz::run(&invalid_edit).unwrap();
    assert_eq!(source, "新文");
    assert_eq!(steps, 1);

    let input = trace("中文🙂", &[(0, u32::MAX, u32::MAX, "新")]);
    let (source, selection, steps) = projection_fuzz::run(&input).unwrap();
    assert_eq!(source, "中文🙂新");
    assert_eq!(selection, 4..4);
    assert_eq!(steps, 1);

    let mut invalid_replacement = trace("A", &[(0, 1, 1, "?")]);
    *invalid_replacement.last_mut().unwrap() = 0xff;
    let (source, selection, _) = projection_fuzz::run(&invalid_replacement).unwrap();
    assert_eq!(source, "A�");
    assert_eq!(selection, 2..2);
}

#[test]
fn projection_trace_replays_standalone_carriage_return_crash() {
    // Nightly libFuzzer artifact crash-911d06f806f25b43073a589c46d835b11bfc5cd5.
    // Initial source is " \u{3}\rV+"; the incomplete edit must not affect it.
    let input = [5, 32, 3, 13, 86, 43, 62, 2, 1, 102, 0];
    let (source, _, steps) = projection_fuzz::run(&input).unwrap();
    assert_eq!(source, " \u{3}\rV+");
    assert_eq!(steps, 0);
}

#[test]
fn projection_trace_replays_fixed_seed_markdown_edit_sequences() {
    // These bounded deterministic traces exercise the same assertions as the
    // fuzz target on every normal test run. They are not a libFuzzer campaign.
    let replacements = ["", "中🙂", "\n", "**", "![x](p)", "\r\n", "\0", "`"];
    for source in [
        "first 中文🙂\n\nlast",
        "**前**![*中*](x)**后**",
        "[![**甲**](a)](outer)*乙*",
        "&NotEqualTilde; and `` ` ``",
        "- [ ] 中\n  - 内\n\n```rust\n****\n```\n\n尾",
        "| 甲 | 乙 |\n| --- | --- |\n| 中 | 🙂 |",
    ] {
        for seed in [7_u32, 29, 113, 65537] {
            let mut state = seed;
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state
            };
            let edits = (0..32)
                .map(|_| {
                    (
                        (next() & 3) as u8,
                        next(),
                        next(),
                        replacements[next() as usize % replacements.len()],
                    )
                })
                .collect::<Vec<_>>();
            let input = trace(source, &edits);
            let outcome = std::panic::catch_unwind(|| projection_fuzz::run(&input)).unwrap_or_else(
                |failure| {
                    let message = failure
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| failure.downcast_ref::<&str>().copied())
                        .unwrap_or("non-string panic");
                    panic!(
                        "projection trace failed: source={source:?} seed={seed} \
                         bytecode={input:?}; {message}"
                    );
                },
            );
            let (_, _, steps) = outcome.unwrap();
            assert_eq!(steps, 32, "source={source:?} seed={seed}");
        }
    }
}
