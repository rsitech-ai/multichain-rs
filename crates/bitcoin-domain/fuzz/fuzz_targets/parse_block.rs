#![no_main]

use bitcoin_domain::parse_block;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = parse_block(bytes);
});
