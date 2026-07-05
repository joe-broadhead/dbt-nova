#![no_main]

use flate2::read::GzDecoder;
use libfuzzer_sys::fuzz_target;
use std::io::Read;
use tar::Archive;

const MAX_ENTRIES: usize = 256;
const MAX_UNCOMPRESSED_BYTES: u64 = 8 * 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    let decoder = GzDecoder::new(data);
    let mut archive = Archive::new(decoder);
    let Ok(entries) = archive.entries() else {
        return;
    };

    let mut entry_count = 0usize;
    let mut uncompressed_bytes = 0u64;
    for entry_result in entries {
        entry_count = entry_count.saturating_add(1);
        if entry_count > MAX_ENTRIES {
            break;
        }
        let Ok(mut entry) = entry_result else {
            continue;
        };
        let Ok(path) = entry.path() else {
            continue;
        };
        for component in path.components() {
            let _ = component.as_os_str();
        }
        let Ok(size) = entry.header().size() else {
            continue;
        };
        uncompressed_bytes = uncompressed_bytes.saturating_add(size);
        if uncompressed_bytes > MAX_UNCOMPRESSED_BYTES {
            break;
        }
        let mut sink = std::io::sink();
        let _ = std::io::copy(&mut entry.by_ref().take(64 * 1024), &mut sink);
    }
});
