use rupora::{
    document::{Document, EditKind},
    markdown::{BlockIndex, analyze, render_html_fragment},
};
use std::time::{Duration, Instant};

fn main() {
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

    let export_source = representative_document(2_000);
    measure("render 2k sections to HTML", Duration::from_secs(2), || {
        std::hint::black_box(render_html_fragment(&export_source));
    });
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
