#![no_main]

use bitcoin_domain::parse_transaction;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = parse_transaction(bytes);
});
