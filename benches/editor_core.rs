use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rupora::markdown::{BlockIndex, analyze, render_html_fragment};
use std::hint::black_box;

fn representative_document(paragraphs: usize) -> String {
    let mut document = String::with_capacity(paragraphs * 96);
    for index in 0..paragraphs {
        document.push_str(&format!(
            "## Section {index}\n\nParagraph {index} with **bold**, [link](note-{index}.md), 中文 and emoji 🦀.\n\n"
        ));
    }
    document
}

fn markdown_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("markdown_analysis");
    for paragraphs in [1_000usize, 10_000] {
        let source = representative_document(paragraphs);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(paragraphs),
            &source,
            |bencher, source| bencher.iter(|| analyze(black_box(source))),
        );
    }
    group.finish();
}

fn incremental_block_reconciliation(c: &mut Criterion) {
    let source = representative_document(10_000);
    let mut edited = source.clone();
    edited.insert_str(edited.len() / 2, "\nSmall local edit.\n");
    c.bench_function("block_index_small_edit_in_large_document", |bencher| {
        bencher.iter_batched(
            || BlockIndex::new(&source),
            |mut index| index.update(black_box(&edited)),
            criterion::BatchSize::SmallInput,
        );
    });
}

fn html_rendering(c: &mut Criterion) {
    let source = representative_document(1_000);
    c.bench_function("html_render_1000_sections", |bencher| {
        bencher.iter(|| render_html_fragment(black_box(&source)));
    });
}

criterion_group!(
    benches,
    markdown_analysis,
    incremental_block_reconciliation,
    html_rendering
);
criterion_main!(benches);
