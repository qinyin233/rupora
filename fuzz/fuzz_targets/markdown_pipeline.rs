#![no_main]

use libfuzzer_sys::fuzz_target;
use rupora::markdown::{BlockIndex, analyze, render_html_document};

fuzz_target!(|data: &[u8]| {
    if data.len() > 1024 * 1024 {
        return;
    }
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let _ = analyze(source);
    let mut blocks = BlockIndex::new(source);
    let updated = format!("{source}\nsmall edit");
    blocks.update(&updated);
    let _ = render_html_document(source, "fuzz", false);
});
