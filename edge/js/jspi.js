// The imports Soli suspends on (see solilang::platform::jspi). Each is wrapped
// in WebAssembly.Suspending: wasm calls it like a plain function, its stack is
// parked while the promise runs, and resumes with the result.

import * as sqlDrivers from "./sql.js";

let host = null;

// worker.js hands over the wasm exports and the Worker's env once per isolate,
// before the first request.
export function setHost(wasm, env) {
  host = { wasm, env };
}

// Called while a future is pending: yield a macrotask, so the fetch that
// future waits on can settle; the stack resumes and polls again.
export const soli_yield = new WebAssembly.Suspending(
  () => new Promise((resolve) => setTimeout(resolve, 0)),
);

// One D1 statement (solilang::db::d1). Reads `{binding, sql, params}` as JSON
// at ptr/len, runs it on that binding, and returns a buffer from soli_alloc:
// a little-endian u32 length, then the JSON response
// `{ok, columns, rows, changes}` or `{ok: false, error}`.
export const soli_d1 = new WebAssembly.Suspending(async (ptr, len) => {
  const { wasm, env } = host;
  let response;
  try {
    const request = JSON.parse(
      new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, ptr, len)),
    );
    const db = env[request.binding];
    if (!db || typeof db.prepare !== "function") {
      throw new Error(
        `no D1 binding named ${request.binding}: add [[d1_databases]] binding = "${request.binding}" to wrangler.toml`,
      );
    }
    const result = await db.prepare(request.sql).bind(...request.params).all();
    const records = result.results ?? [];
    const columns = records.length ? Object.keys(records[0]) : [];
    response = {
      ok: true,
      columns,
      rows: records.map((record) => columns.map((c) => cell(record[c]))),
      changes: result.meta?.changes ?? 0,
    };
  } catch (error) {
    response = { ok: false, error: String(error?.message ?? error) };
  }
  // Allocate only now: soli_alloc may grow memory and detach older views.
  const bytes = new TextEncoder().encode(JSON.stringify(response));
  const out = wasm.soli_alloc(4 + bytes.length);
  const view = new DataView(wasm.memory.buffer);
  view.setUint32(out, bytes.length, true);
  new Uint8Array(wasm.memory.buffer, out + 4, bytes.length).set(bytes);
  return out;
});

// D1 hands back strings, numbers, null and ArrayBuffers; JSON has no bytes.
function cell(value) {
  if (value instanceof ArrayBuffer) return `<blob ${value.byteLength} bytes>`;
  return value ?? null;
}

// One R2 operation (solilang's `r2` attachment service). Reads the JSON head
// `{op, binding, key, content_type?, filename?}` at headPtr/headLen and, for a
// put, the bytes at bodyPtr/bodyLen. Returns a buffer from soli_alloc: a
// little-endian u32 head length, a u32 body length, the JSON head
// `{ok, found, content_type, size, filename}` or `{ok: false, error}`, then
// the object's bytes for a get.
export const soli_r2 = new WebAssembly.Suspending(async (headPtr, headLen, bodyPtr, bodyLen) => {
  const { wasm, env } = host;
  let response;
  let bytes = new Uint8Array(0);
  try {
    const request = JSON.parse(
      new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, headPtr, headLen)),
    );
    const bucket = env[request.binding];
    if (!bucket || typeof bucket.head !== "function") {
      throw new Error(
        `no R2 binding named ${request.binding}: add [[r2_buckets]] binding = "${request.binding}" to wrangler.toml`,
      );
    }
    switch (request.op) {
      case "put": {
        // A copy: wasm memory may grow, and detach this view, before R2 reads it.
        const data = new Uint8Array(wasm.memory.buffer, bodyPtr, bodyLen).slice();
        await bucket.put(request.key, data, {
          httpMetadata: { contentType: request.content_type },
          customMetadata: { "original-filename": request.filename },
        });
        response = { ok: true, found: true };
        break;
      }
      case "get": {
        const object = await bucket.get(request.key);
        if (object) bytes = new Uint8Array(await object.arrayBuffer());
        response = described(object);
        break;
      }
      case "head":
        response = described(await bucket.head(request.key));
        break;
      case "delete":
        await bucket.delete(request.key);
        response = { ok: true, found: true };
        break;
      default:
        throw new Error(`unknown R2 operation ${request.op}`);
    }
  } catch (error) {
    response = { ok: false, error: String(error?.message ?? error) };
    bytes = new Uint8Array(0);
  }
  // Allocate only now: soli_alloc may grow memory and detach older views.
  const head = new TextEncoder().encode(JSON.stringify(response));
  const out = wasm.soli_alloc(8 + head.length + bytes.length);
  const view = new DataView(wasm.memory.buffer);
  view.setUint32(out, head.length, true);
  view.setUint32(out + 4, bytes.length, true);
  const buffer = new Uint8Array(wasm.memory.buffer, out + 8, head.length + bytes.length);
  buffer.set(head);
  buffer.set(bytes, head.length);
  return out;
});

function described(object) {
  if (!object) return { ok: true, found: false };
  return {
    ok: true,
    found: true,
    content_type: object.httpMetadata?.contentType ?? "application/octet-stream",
    size: object.size,
    filename: object.customMetadata?.["original-filename"] ?? "file",
  };
}

// The request's SQL connections, one per target, opened on first use so every
// statement of a request — a transaction's too — runs on the same one.
const sqlClients = new Map();

// worker.js calls this when a request is done: its connections close (with
// Hyperdrive that hands them back to the pool) and an open transaction rolls
// back with them.
export function endSql(ctx) {
  for (const pending of sqlClients.values()) {
    const closing = pending.then((client) => client.end()).catch(() => {});
    ctx?.waitUntil?.(closing);
  }
  sqlClients.clear();
}

function connectionString(target) {
  if (!target.startsWith("hyperdrive://")) return target;
  const name = target.slice("hyperdrive://".length);
  const binding = host.env[name];
  if (!binding || typeof binding.connectionString !== "string") {
    throw new Error(
      `no Hyperdrive binding named ${name}: add [[hyperdrive]] binding = "${name}" to wrangler.toml`,
    );
  }
  return binding.connectionString;
}

// One statement for Soli's postgres adapter (solilang::db::hyperdrive). Reads
// `{target, sql, params}` as JSON at ptr/len, runs it with the `pg` driver
// `soli edge build` bundled, and returns a buffer from soli_alloc: a
// little-endian u32 length, then the JSON response `{ok, columns, rows,
// changes}` or `{ok: false, error}`.
export const soli_sql = new WebAssembly.Suspending(async (ptr, len) => {
  const { wasm } = host;
  let response;
  try {
    const request = JSON.parse(
      new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, ptr, len)),
    );
    const connect = sqlDrivers.postgres;
    if (!connect) {
      throw new Error(
        "this Worker carries no postgres driver: rebuild with `soli edge build --sql postgres`",
      );
    }
    let pending = sqlClients.get(request.target);
    if (!pending) {
      pending = connect(connectionString(request.target));
      sqlClients.set(request.target, pending);
      pending.catch(() => sqlClients.delete(request.target));
    }
    const client = await pending;
    const result = await client.query(request.sql, request.params);
    response = {
      ok: true,
      columns: result.columns,
      rows: result.rows.map((row) => row.map(sqlCell)),
      changes: result.changes,
    };
  } catch (error) {
    response = { ok: false, error: String(error?.message ?? error) };
  }
  const bytes = new TextEncoder().encode(JSON.stringify(response));
  const out = wasm.soli_alloc(4 + bytes.length);
  new DataView(wasm.memory.buffer).setUint32(out, bytes.length, true);
  new Uint8Array(wasm.memory.buffer, out + 4, bytes.length).set(bytes);
  return out;
});

// JSON has no bigint, Date or bytes.
function sqlCell(value) {
  if (typeof value === "bigint") return value.toString();
  if (value instanceof Date) return value.toISOString();
  if (value instanceof Uint8Array) return `<blob ${value.byteLength} bytes>`;
  return value ?? null;
}

// Send one job to a Cloudflare Queue (solilang::jobs::queues). Reads
// `{binding, body, delay_seconds}` as JSON at ptr/len and returns a buffer
// from soli_alloc: a little-endian u32 length, then `{ok}` or `{ok: false,
// error}`.
export const soli_queue_send = new WebAssembly.Suspending(async (ptr, len) => {
  const { wasm, env } = host;
  let response;
  try {
    const request = JSON.parse(
      new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, ptr, len)),
    );
    const queue = env[request.binding];
    if (!queue || typeof queue.send !== "function") {
      throw new Error(
        `no Queue binding named ${request.binding}: add [[queues.producers]] binding = "${request.binding}" to wrangler.toml`,
      );
    }
    const options = request.delay_seconds > 0 ? { delaySeconds: request.delay_seconds } : {};
    await queue.send(request.body, options);
    response = { ok: true };
  } catch (error) {
    response = { ok: false, error: String(error?.message ?? error) };
  }
  const bytes = new TextEncoder().encode(JSON.stringify(response));
  const out = wasm.soli_alloc(4 + bytes.length);
  new DataView(wasm.memory.buffer).setUint32(out, bytes.length, true);
  new Uint8Array(wasm.memory.buffer, out + 4, bytes.length).set(bytes);
  return out;
});
