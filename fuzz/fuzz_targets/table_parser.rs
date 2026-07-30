#![no_main]

use libfuzzer_sys::fuzz_target;
use rupora::table;

fuzz_target!(|data: &[u8]| {
    if data.len() > 1024 * 1024 {
        return;
    }
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    for cursor in [0, source.len() / 2, source.len()] {
        if let Some(table) = table::find_table(source, cursor) {
            let markdown = table.to_markdown();
            let _ = table::find_table(&markdown, 0);
        }
    }
});
