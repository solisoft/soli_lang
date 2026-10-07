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
    // parser and reporting clean. (That parser, sonic-rs, is gone.)
    if let Ok(value) = solilang::interpreter::value::parse_json(text) {
        let _ = value_to_json(&value);
        // The writer behind `render_json` / `JSON.stringify`: whatever the
        // parser accepted must be written back as JSON the parser accepts.
        if let Ok(written) = solilang::interpreter::value::stringify_to_string(&value) {
            solilang::interpreter::value::parse_json(&written)
                .expect("the JSON writer's output parses");
        }
    }
});
