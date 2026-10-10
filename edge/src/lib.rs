//! The wasm side of a Soli Worker. The JS host (`js/worker.js`) mounts the
//! app's files and environment, boots once per isolate, then hands every
//! request to `soli_handle`, and every Cloudflare Queue message to
//! `soli_run_job`.
//!
//! `soli_handle` is a raw export rather than a wasm-bindgen one: the host enters
//! it through `WebAssembly.promising`, so that a model query inside the action
//! can suspend the wasm stack on its `fetch` (see `solilang::platform::jspi`).
//! wasm-bindgen's glue calls exports synchronously and could not await it.

use solilang::serve::edge::{self, EdgeRequest};
use wasm_bindgen::prelude::*;

/// Mount the app: `files_json` is `[[absolute path, contents], ...]`.
#[wasm_bindgen]
pub fn mount(files_json: &str) -> Result<(), String> {
    console_error_panic_hook::set_once();
    let files: Vec<(String, String)> =
        serde_json::from_str(files_json).map_err(|e| format!("mount: {e}"))?;
    solilang::platform::fs::mount(
        files
            .into_iter()
            .map(|(path, contents)| (path, contents.into_bytes()))
            .collect(),
    );
    Ok(())
}

/// The process environment, from the Worker's string bindings: `[[name, value], ...]`,
/// plus `SOLI_RUNTIME` and `SOLI_VERSION`.
#[wasm_bindgen]
pub fn set_env(vars_json: &str) -> Result<(), String> {
    let mut vars: Vec<(String, String)> =
        serde_json::from_str(vars_json).map_err(|e| format!("set_env: {e}"))?;
    // What an app reads to know where it runs, unless the Worker says otherwise.
    for (name, value) in [
        ("SOLI_RUNTIME", "cloudflare-workers"),
        ("SOLI_VERSION", edge::VERSION),
    ] {
        if !vars.iter().any(|(n, _)| n == name) {
            vars.push((name.to_string(), value.to_string()));
        }
    }
    solilang::platform::env::set(vars);
    Ok(())
}

#[wasm_bindgen]
pub fn boot(root: &str) -> Result<(), String> {
    edge::boot(std::path::Path::new(root)).map_err(|e| e.to_string())
}

thread_local! {
    static RESPONSE: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A buffer of `len` bytes for the host to write a request into; ownership
/// passes to `soli_handle`.
#[no_mangle]
pub extern "C" fn soli_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// Handle the request at `ptr` (from `soli_alloc`): a little-endian `u32`
/// length, that many bytes of JSON head (`{method, path, query, headers}`),
/// then the raw body. Returns a pointer to the response, `soli_response_len`
/// bytes long and valid until the next call, in the same frame: a `u32`
/// length, the JSON head (`{status, headers}`), then the raw body.
///
/// The bodies travel as bytes, not inside the JSON: an uploaded photo, or an
/// image the app answers with, is not UTF-8.
#[no_mangle]
pub extern "C" fn soli_handle(ptr: *mut u8, len: usize) -> *const u8 {
    // SAFETY: `ptr`/`len` describe the buffer `soli_alloc(len)` returned, which
    // the host filled and hands back exactly once.
    let input = unsafe { Vec::from_raw_parts(ptr, len, len) };
    let response = match split_frame(input) {
        Ok((json, body)) => {
            let field = |key: &str| json[key].as_str().unwrap_or_default().to_string();
            edge::handle(EdgeRequest {
                method: field("method"),
                path: field("path"),
                query: field("query"),
                headers: serde_json::from_value(json["headers"].clone()).unwrap_or_default(),
                body,
            })
        }
        Err(e) => edge::EdgeResponse {
            status: 400,
            headers: Vec::new(),
            body: format!("Malformed request from the host: {e}").into_bytes(),
        },
    };
    let head = serde_json::json!({
        "status": response.status,
        "headers": response.headers,
    })
    .to_string()
    .into_bytes();
    let mut output = Vec::with_capacity(4 + head.len() + response.body.len());
    output.extend_from_slice(&(head.len() as u32).to_le_bytes());
    output.extend_from_slice(&head);
    output.extend_from_slice(&response.body);
    RESPONSE.with(|cell| {
        *cell.borrow_mut() = output;
        cell.borrow().as_ptr()
    })
}

/// Run the job of one Cloudflare Queue message: the JSON at `ptr` (from
/// `soli_alloc`) is `{message, attempts}`, `message` the job's JSON. Returns a
/// pointer to the JSON report (`{status: "done" | "retry" | "dead", delay?,
/// error?, job}`), `soli_response_len` bytes long and valid until the next call.
/// Entered through `WebAssembly.promising`, like `soli_handle`: the job's
/// queries suspend the stack.
#[no_mangle]
pub extern "C" fn soli_run_job(ptr: *mut u8, len: usize) -> *const u8 {
    // SAFETY: as in `soli_handle`.
    let input = unsafe { Vec::from_raw_parts(ptr, len, len) };
    let report = match serde_json::from_slice::<serde_json::Value>(&input) {
        Ok(json) => edge::run_job(
            json["message"].as_str().unwrap_or_default(),
            json["attempts"].as_u64().unwrap_or(1) as u32,
        ),
        Err(e) => serde_json::json!({
            "status": "dead",
            "error": format!("Malformed job from the host: {e}"),
        }),
    };
    RESPONSE.with(|cell| {
        *cell.borrow_mut() = report.to_string().into_bytes();
        cell.borrow().as_ptr()
    })
}

/// A frame's JSON head and the body after it. The body keeps the input's
/// allocation: an upload is not copied a second time.
fn split_frame(mut input: Vec<u8>) -> Result<(serde_json::Value, Vec<u8>), String> {
    let head_len = input
        .get(..4)
        .map(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize)
        .ok_or("no head length")?;
    let head_end = 4usize
        .checked_add(head_len)
        .filter(|end| *end <= input.len())
        .ok_or("head longer than the frame")?;
    let json = serde_json::from_slice(&input[4..head_end]).map_err(|e| e.to_string())?;
    input.drain(..head_end);
    Ok((json, input))
}

#[no_mangle]
pub extern "C" fn soli_response_len() -> usize {
    RESPONSE.with(|cell| cell.borrow().len())
}
