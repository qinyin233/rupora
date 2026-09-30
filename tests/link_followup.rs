use rupora::{
    editing::{MarkdownCommand, apply_markdown_command},
    wysiwyg::VisualProjection,
};

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
