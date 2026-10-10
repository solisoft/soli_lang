// A Soli app as a Cloudflare Worker, written by `soli edge build`.
//
// The app's files ship inside the bundle (app.json); each isolate boots them
// once and reuses them across requests. Requests enter wasm through
// `WebAssembly.promising`, so a model query can suspend the stack on its fetch
// (jspi.js) — a D1 or Postgres query too. Files under public/ are served by Workers static assets before
// the request ever reaches this script.
import wasmModule from "./soli_edge_bg.wasm";
import { initSync, mount, set_env, boot } from "./soli_edge.js";
import { setHost, endSql } from "./jspi.js";
import appFiles from "./app.json";

let wasm = null;
let handleRequest = null;
let bootError = null;
// One request inside wasm at a time: a suspended stack still owns its part of
// the shadow stack in linear memory, which a second request would overwrite.
let queue = Promise.resolve();

const encoder = new TextEncoder();
const decoder = new TextDecoder();

// Set by AttachmentsController on an R2 blob: this script answers from the
// bucket, so the bytes never cross wasm.
const R2_OBJECT = "x-soli-r2-object";

function ensureBooted(env) {
  if (wasm || bootError) return;
  try {
    wasm = initSync({ module: wasmModule });
    handleRequest = WebAssembly.promising(wasm.soli_handle);
    // The D1 and R2 imports read and write wasm memory and look bindings up in env.
    setHost(wasm, env);
    set_env(JSON.stringify(Object.entries(env).filter(([, value]) => typeof value === "string")));
    mount(JSON.stringify(appFiles));
    boot("/app");
  } catch (error) {
    bootError = String(error);
  }
}

// Both ways, a frame: a little-endian u32 length, that many bytes of JSON head,
// then the raw body — uploads and images are not UTF-8.
async function dispatch(head, body) {
  const headBytes = encoder.encode(JSON.stringify(head));
  const length = 4 + headBytes.length + body.length;
  const ptr = wasm.soli_alloc(length);
  new DataView(wasm.memory.buffer).setUint32(ptr, headBytes.length, true);
  const input = new Uint8Array(wasm.memory.buffer, ptr, length);
  input.set(headBytes, 4);
  input.set(body, 4 + headBytes.length);
  const out = await handleRequest(ptr, length);
  const total = wasm.soli_response_len();
  const headLength = new DataView(wasm.memory.buffer).getUint32(out, true);
  const result = JSON.parse(
    decoder.decode(new Uint8Array(wasm.memory.buffer, out + 4, headLength)),
  );
  // A copy: the response buffer is reused by the next request.
  result.body = new Uint8Array(wasm.memory.buffer, out + 4 + headLength, total - 4 - headLength).slice();
  return result;
}

// Statuses whose Response must not have a body, even an empty one.
function bodyless(request, status) {
  return request.method === "HEAD" || [101, 204, 205, 304].includes(status);
}

// The R2 object AttachmentsController named, with the headers it set (type,
// disposition, nosniff, cache). Range and If-None-Match are R2's to answer; an
// image transform goes through the Images binding when there is one, and falls
// back to the stored bytes when there is none or it fails — as under
// `soli serve`.
async function serveObject(request, env, result, reference) {
  const bucket = env[reference.binding];
  if (!bucket || typeof bucket.get !== "function") {
    return new Response(
      `no R2 binding named ${reference.binding}: add [[r2_buckets]] binding = "${reference.binding}" to wrangler.toml`,
      { status: 500 },
    );
  }
  // `set`, not `append`: the controller and the response layer both send
  // X-Content-Type-Options, and one is enough.
  const headers = new Headers();
  for (const [name, value] of result.headers) {
    if (name.toLowerCase() !== R2_OBJECT) headers.set(name, value);
  }

  if (reference.image && env.IMAGES && typeof env.IMAGES.input === "function") {
    const object = await bucket.get(reference.key);
    if (!object) return new Response("Not found", { status: 404 });
    try {
      const image = await env.IMAGES.input(object.body)
        .transform(reference.image.transform)
        .output(reference.image.output);
      headers.set("content-type", image.contentType());
      headers.set("cache-control", "public, max-age=86400");
      const response = image.response();
      return new Response(bodyless(request, 200) ? null : response.body, { status: 200, headers });
    } catch (error) {
      console.error(`Images binding: ${error?.message ?? error}`);
      headers.set("cache-control", "private, max-age=60");
    }
  }

  let object;
  try {
    object = await bucket.get(reference.key, { range: request.headers, onlyIf: request.headers });
  } catch (error) {
    // An unsatisfiable Range.
    return new Response("Range Not Satisfiable", { status: 416 });
  }
  if (!object) return new Response("Not found", { status: 404 });
  headers.set("etag", object.httpEtag);
  headers.set("accept-ranges", "bytes");
  // onlyIf failed: the client's copy is current.
  if (!("body" in object)) return new Response(null, { status: 304, headers });

  let status = 200;
  let length = object.size;
  const range = object.range;
  if (request.headers.has("range") && range && (range.offset !== undefined || range.suffix !== undefined)) {
    const start = range.suffix !== undefined ? Math.max(0, object.size - range.suffix) : range.offset;
    length = Math.min(range.length ?? object.size - start, object.size - start);
    headers.set("content-range", `bytes ${start}-${start + length - 1}/${object.size}`);
    status = 206;
  }
  headers.set("content-length", String(length));
  return new Response(bodyless(request, status) ? null : object.body, { status, headers });
}

export default {
  async fetch(request, env, ctx) {
    ensureBooted(env);
    if (bootError) {
      return new Response(`Soli failed to boot: ${bootError}`, { status: 500 });
    }
    const url = new URL(request.url);
    const headers = [...request.headers];
    // Where Cloudflare ran this request: request.cf, which is not a header.
    if (request.cf?.colo) headers.push(["cf-colo", request.cf.colo]);
    const body = ["GET", "HEAD"].includes(request.method)
      ? new Uint8Array(0)
      : new Uint8Array(await request.arrayBuffer());
    // The request's SQL connections close before the next request enters wasm.
    const job = queue
      .then(() =>
        dispatch(
          {
            method: request.method,
            path: url.pathname,
            query: url.search.slice(1),
            headers,
          },
          body,
        ),
      )
      .finally(() => endSql(ctx));
    queue = job.catch(() => {});
    const result = await job;
    const object = result.headers.find(([name]) => name.toLowerCase() === R2_OBJECT);
    if (object && result.status === 200) {
      return serveObject(request, env, result, JSON.parse(object[1]));
    }
    return new Response(bodyless(request, result.status) ? null : result.body, {
      status: result.status,
      headers: result.headers,
    });
  },
};
