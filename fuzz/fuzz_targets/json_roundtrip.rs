#![no_main]
//! Fuzz JSON parsing and Value conversion in both directions. Any input must
//! produce a value or a clean error — never a panic or stack overflow.

use libfuzzer_sys::fuzz_target;
use solilang::interpreter::value_json::value_to_json;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(value) = solilang::interpreter::value_json::parse_json_sonic(text) {
        // Round-trip through the Soli Value representation.
        let _ = value_to_json(&value);
    }
});
