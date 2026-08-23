#![no_main]
//! Fuzz JSON parsing and Value conversion in both directions. Any input must
//! produce a value or a clean error — never a panic or stack overflow.

use libfuzzer_sys::fuzz_target;
use solilang::interpreter::value_json::value_to_json;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    // The hand-rolled parser FIRST: this is what `json_parse` and the request
    // body behind `req["json"]` actually call, and it is the one that mattered.
    // This target used to fuzz only `parse_json_sonic`, so a 100k-deep array
    // overflowed the stack in production code while the fuzzer — whose whole
    // promise is "never a panic or stack overflow" — was exercising a different
    // parser and reporting clean.
    if let Ok(value) = solilang::interpreter::value::parse_json(text) {
        let _ = value_to_json(&value);
    }

    // The SIMD path, on the same input, so the two parsers keep getting the
    // same corpus.
    if let Ok(value) = solilang::interpreter::value_json::parse_json_sonic(text) {
        let _ = value_to_json(&value);
    }
});
