// The imports Soli suspends on (see solilang::platform::jspi). Each is wrapped
// in WebAssembly.Suspending: wasm calls it like a plain function, its stack is
// parked while the promise runs, and resumes with the result.

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
