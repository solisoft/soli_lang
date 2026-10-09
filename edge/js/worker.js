// A Soli app as a Cloudflare Worker, written by `soli edge build`.
//
// The app's files ship inside the bundle (app.json); each isolate boots them
// once and reuses them across requests. Requests enter wasm through
// `WebAssembly.promising`, so a model query can suspend the stack on its fetch
// (jspi.js). Files under public/ are served by Workers static assets before
// the request ever reaches this script.
import wasmModule from "./soli_edge_bg.wasm";
import { initSync, mount, set_env, boot } from "./soli_edge.js";
import appFiles from "./app.json";

let wasm = null;
let handleRequest = null;
let bootError = null;
// One request inside wasm at a time: a suspended stack still owns its part of
// the shadow stack in linear memory, which a second request would overwrite.
let queue = Promise.resolve();

function ensureBooted(env) {
  if (wasm || bootError) return;
  try {
    wasm = initSync({ module: wasmModule });
    handleRequest = WebAssembly.promising(wasm.soli_handle);
    set_env(JSON.stringify(Object.entries(env).filter(([, value]) => typeof value === "string")));
    mount(JSON.stringify(appFiles));
    boot("/app");
  } catch (error) {
    bootError = String(error);
  }
}

async function dispatch(request) {
  const input = new TextEncoder().encode(JSON.stringify(request));
  const ptr = wasm.soli_alloc(input.length);
  new Uint8Array(wasm.memory.buffer, ptr, input.length).set(input);
  const out = await handleRequest(ptr, input.length);
  const output = new Uint8Array(wasm.memory.buffer, out, wasm.soli_response_len());
  return JSON.parse(new TextDecoder().decode(output));
}

export default {
  async fetch(request, env) {
    ensureBooted(env);
    if (bootError) {
      return new Response(`Soli failed to boot: ${bootError}`, { status: 500 });
    }
    const url = new URL(request.url);
    const headers = [...request.headers];
    // Where Cloudflare ran this request: request.cf, which is not a header.
    if (request.cf?.colo) headers.push(["cf-colo", request.cf.colo]);
    const body = ["GET", "HEAD"].includes(request.method) ? "" : await request.text();
    const job = queue.then(() =>
      dispatch({
        method: request.method,
        path: url.pathname,
        query: url.search.slice(1),
        headers,
        body,
      }),
    );
    queue = job.catch(() => {});
    const result = await job;
    return new Response(result.body, { status: result.status, headers: result.headers });
  },
};
