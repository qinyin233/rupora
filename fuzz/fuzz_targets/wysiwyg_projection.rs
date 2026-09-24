#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/wysiwyg_projection.rs"]
mod wysiwyg_projection;

fuzz_target!(|data: &[u8]| {
    let _ = wysiwyg_projection::run(data);
});
