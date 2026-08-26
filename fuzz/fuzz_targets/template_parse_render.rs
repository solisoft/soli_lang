#![no_main]
//! Fuzz the template engine: tokenize, parse, and render with empty locals.
//! Any input must produce output or a clean error — never a panic or stack
//! overflow. Partial includes are stubbed (no filesystem in the fuzzer).

use libfuzzer_sys::fuzz_target;
use solilang::interpreter::value::Value;
use solilang::template::parser::parse_template;
use solilang::template::renderer::render_nodes_with_path;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(nodes) = parse_template(source) else {
        return;
    };
    // `<%= ... %>` is handed to the *core* language parser and interpreter, so a
    // backtick inside a tag is command substitution: rendering it spawns
    // `sh -c <fuzzer bytes>` on a detached thread and returns a Future the
    // renderer drops unread — arbitrary shell, from fuzzer bytes, on the host.
    // Rendering is meant to be side-effect free here (partials are already
    // stubbed out), so skip any input that can reach it. Parsing such a template
    // is still fuzzed above — only the render is skipped, and only for the whole
    // input, since the cheap check cannot tell a backtick in an ERB tag from one
    // in body text.
    //
    // This skip also used to be what kept LeakSanitizer from failing the run on
    // the in-flight thread's allocations. Leak detection is off now
    // (`ASAN_OPTIONS=detect_leaks=0` in `.github/workflows/fuzz.yml`), but the
    // shell-execution reason above stands on its own — do not drop the guard.
    if source.contains('`') {
        return;
    }
    let no_partials: Option<&dyn Fn(&str, &Value) -> Result<String, String>> = None;
    let _ = render_nodes_with_path(&nodes, &Value::Null, no_partials, None);
});
