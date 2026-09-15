use rupora::{
    document::{Document, EditKind},
    editing::replace_all,
    markdown::{BlockIndex, analyze, render_html_fragment},
    wysiwyg::VisualProjection,
};
use std::time::{Duration, Instant};

fn main() {
    let rich_paragraph = "`中` ".repeat(128_000);
    measure(
        "project 128k inline code spans",
        Duration::from_secs(2),
        || {
            let projection = VisualProjection::from_markdown(&rich_paragraph);
            assert_eq!(projection.text(), "中 ".repeat(128_000));
            std::hint::black_box(projection);
        },
    );

    let source = representative_document(20_000);
    measure("analyze 20k sections", Duration::from_secs(8), || {
        std::hint::black_box(analyze(&source));
    });

    let mut index = BlockIndex::new(&source);
    let mut edited = source.clone();
    edited.insert_str(edited.len() / 2, "\nA local edit.\n");
    measure("reconcile 20k blocks", Duration::from_secs(1), || {
        index.update(&edited);
    });

    let mut document = Document::untitled(1);
    document.content = source.clone();
    document.update_after_edit();
    let before = document.content.clone();
    document.content.push('!');
    measure(
        "record edit without synchronous analysis",
        Duration::from_millis(250),
        || {
            assert!(document.record_edit(before, None, None, EditKind::Typing));
            assert!(document.derived_state_is_stale());
        },
    );

    measure(
        "read updated blocks without forcing analysis",
        Duration::from_secs(1),
        || {
            std::hint::black_box(document.blocks());
            assert!(document.derived_state_is_stale());
        },
    );
    measure(
        "read unchanged blocks 1000 times during deferred analysis",
        Duration::from_millis(100),
        || {
            for _ in 0..1_000 {
                std::hint::black_box(document.blocks());
            }
            assert!(document.derived_state_is_stale());
        },
    );

    let export_source = representative_document(2_000);
    measure(
        "atomic command on 20k sections",
        Duration::from_millis(250),
        || {
            assert!(document.edit(EditKind::Format, None, |text| {
                text.push_str("\nNew paragraph.\n");
                None
            }));
            assert!(document.derived_state_is_stale());
        },
    );
    measure("render 2k sections to HTML", Duration::from_secs(2), || {
        std::hint::black_box(render_html_fragment(&export_source));
    });

    let mut repeated = "中文 Aa 🙂\n".repeat(25_000);
    measure(
        "replace 25k Unicode matches",
        Duration::from_secs(1),
        || {
            assert_eq!(replace_all(&mut repeated, "aa", "替换", false), 25_000);
        },
    );
}

fn measure(name: &str, budget: Duration, operation: impl FnOnce()) {
    let started = Instant::now();
    operation();
    let elapsed = started.elapsed();
    println!("{name}: {:.3}s", elapsed.as_secs_f64());
    assert!(
        elapsed <= budget,
        "{name} exceeded the {:?} performance budget: {:?}",
        budget,
        elapsed
    );
}

fn representative_document(paragraphs: usize) -> String {
    let mut document = String::with_capacity(paragraphs * 96);
    for index in 0..paragraphs {
        document.push_str(&format!(
            "## Section {index}\n\nParagraph {index} with **bold**, [link](note-{index}.md), 中文 and emoji 🦀.\n\n"
        ));
    }
    document
}
