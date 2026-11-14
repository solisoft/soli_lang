// A Soli app as a Cloudflare Worker, written by `soli edge build`.
//
// The app's files ship inside the bundle (app.json); each isolate boots them
// once and reuses them across requests. Requests enter wasm through
// `WebAssembly.promising`, so a model query can suspend the stack on its fetch
// (jspi.js) — a D1 or Postgres query too. Files under public/ are served by Workers static assets before
// the request ever reaches this script. Jobs enqueued on the edge come back
// through `queue()` as Cloudflare Queue messages.
import { DurableObject } from "cloudflare:workers";
import wasmModule from "./soli_edge_bg.wasm";
import { initSync, mount, set_env, boot } from "./soli_edge.js";
import { setHost, endSql } from "./jspi.js";
import appFiles from "./app.json";
// The Durable Object a `do://BINDING/name` connection reaches (wrangler.toml
// binds it by this class name).
export { SoliSqlObject } from "./durable.js";

let wasm = null;
let handleRequest = null;
let runJob = null;
let runLive = null;
let runCron = null;
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
    runJob = WebAssembly.promising(wasm.soli_run_job);
    runLive = WebAssembly.promising(wasm.soli_live);
    runCron = WebAssembly.promising(wasm.soli_run_cron);
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

// One queued job through wasm: the report says what to do with its message.
async function dispatchJob(message) {
  const input = encoder.encode(
    JSON.stringify({ message: JSON.stringify(message.body), attempts: message.attempts }),
  );
  const ptr = wasm.soli_alloc(input.length);
  new Uint8Array(wasm.memory.buffer, ptr, input.length).set(input);
  const out = await runJob(ptr, input.length);
  return JSON.parse(
    decoder.decode(new Uint8Array(wasm.memory.buffer, out, wasm.soli_response_len())),
  );
}

// One LiveView frame through wasm (`edge::live_frame`), behind any request
// inside wasm: `{instance, messages, error?}`.
function liveFrame(input, ctx) {
  const job = queue
    .then(async () => {
      const bytes = encoder.encode(JSON.stringify(input));
      const ptr = wasm.soli_alloc(bytes.length);
      new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
      const out = await runLive(ptr, bytes.length);
      return JSON.parse(
        decoder.decode(new Uint8Array(wasm.memory.buffer, out, wasm.soli_response_len())),
      );
    })
    .finally(() => endSql(ctx));
  queue = job.catch(() => {});
  return job;
}

// A LiveView lives in a Durable Object: one object per instance (a session's,
// or a room's), which holds the sockets and keeps the instance in its storage.
// Sockets are accepted with the hibernation API, so the object — and its
// isolate — can be evicted between events: each event reads the instance
// back, runs one frame and stores the result.
//
// wrangler.toml:
//   [[durable_objects.bindings]]
//   name = "SOLI_LIVE"
//   class_name = "SoliLiveObject"
//   [[migrations]]
//   tag = "soli-live-v1"
//   new_sqlite_classes = ["SoliLiveObject"]
export class SoliLiveObject extends DurableObject {
  async fetch(request) {
    ensureBooted(this.env);
    if (bootError) return new Response(`Soli failed to boot: ${bootError}`, { status: 500 });
    const meta = {
      component: request.headers.get("x-soli-live-component"),
      session: request.headers.get("x-soli-live-session"),
      room: request.headers.get("x-soli-live-room") || undefined,
    };
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server);
    server.serializeAttachment(meta);

    const stored = await this.ctx.storage.get("instance");
    if (stored) {
      // Already mounted (another tab, a reconnect, a room's next visitor):
      // its current markup to this socket only.
      server.send(JSON.stringify({ type: "Render", html: stored.last_html, liveview_id: stored.id }));
    } else {
      await this.frame({ op: "mount", ...meta }, null);
    }
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws, data) {
    ensureBooted(this.env);
    let message;
    try {
      message = JSON.parse(typeof data === "string" ? data : decoder.decode(data));
    } catch {
      return;
    }
    if (message.type === "heartbeat") {
      ws.send(JSON.stringify({ type: "heartbeat_ack" }));
      return;
    }
    const instance = await this.ctx.storage.get("instance");
    if (!instance) return;
    if (message.type === "resync") {
      ws.send(JSON.stringify({ type: "Render", html: instance.last_html, liveview_id: instance.id }));
      return;
    }
    if (message.type !== "event" || typeof message.event !== "string") return;
    const meta = ws.deserializeAttachment() || {};
    await this.frame(
      { op: "event", instance, event: message.event, params: message.params || {}, session: meta.session },
      ws,
    );
  }

  webSocketClose(ws, code) {
    // 1005 (no status) and 1006 (abnormal) describe a close; they cannot be sent.
    try {
      ws.close(code === 1005 || code === 1006 ? 1000 : code);
    } catch {}
  }

  // Run a frame, keep the instance, send its messages to every socket of this
  // LiveView (or report an error to the one that sent the event).
  async frame(input, from) {
    const started = Date.now();
    const result = await liveFrame(input, this.ctx);
    if (result.instance) await this.ctx.storage.put("instance", result.instance);
    for (const socket of this.ctx.getWebSockets()) {
      for (const message of result.messages || []) socket.send(message);
    }
    if (result.error) {
      console.error(`LiveView ${input.op}: ${result.error}`);
      if (from) from.send(JSON.stringify({ type: "Error", message: result.error }));
    }
    console.log(`LiveView ${input.op} ${input.event || input.component}: ${Date.now() - started} ms`);
  }
}

// `/live/socket/<component>`: the Durable Object of that LiveView instance —
// the room's when `?room=` names one, else the visitor's session's.
async function liveSocket(request, env, ctx, url) {
  if (request.headers.get("upgrade")?.toLowerCase() !== "websocket") {
    return new Response("Expected a WebSocket upgrade", { status: 426 });
  }
  if (!env.SOLI_LIVE) {
    return new Response(
      'LiveView on the edge needs a Durable Object binding: add [[durable_objects.bindings]] name = "SOLI_LIVE", class_name = "SoliLiveObject" to wrangler.toml',
      { status: 501 },
    );
  }
  const component = decodeURIComponent(url.pathname.slice("/live/socket/".length));
  if (!/^[A-Za-z0-9_-]+$/.test(component)) return new Response("Bad component", { status: 400 });
  const room = url.searchParams.get("room");
  const cookie = request.headers.get("cookie") || "";
  const session =
    /(?:^|;\s*)(?:__Host-)?session_id=([^;]+)/.exec(cookie)?.[1] || `sess-${crypto.randomUUID()}`;
  // The instance id by the server's rules: a room only for a component
  // `live_rooms` declared. One Durable Object per instance.
  const { id: name } = await liveFrame({ op: "id", component, session, room: room || undefined }, ctx);
  const headers = new Headers(request.headers);
  headers.set("x-soli-live-component", component);
  headers.set("x-soli-live-session", session);
  if (room) headers.set("x-soli-live-room", room);
  const stub = env.SOLI_LIVE.get(env.SOLI_LIVE.idFromName(name));
  return stub.fetch(new Request(request, { headers }));
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
    if (url.pathname.startsWith("/live/socket/")) return liveSocket(request, env, ctx, url);
    const headers = [...request.headers];
    // Where Cloudflare ran this request: request.cf, which is not a header.
    if (request.cf?.colo) headers.push(["cf-colo", request.cf.colo]);
    // The edge adds cf-ipcountry (when the zone's IP geolocation is on);
    // wrangler dev does not, but fills request.cf.country.
    if (request.cf?.country && !request.headers.has("cf-ipcountry")) {
      headers.push(["cf-ipcountry", request.cf.country]);
    }
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

  // Cron Triggers (`[triggers] crons` in wrangler.toml, which `soli edge build`
  // writes from the jobs' `static cron`): the jobs whose schedule this is run
  // in turn, behind any request inside wasm. A failed one fails the
  // invocation, so the dashboard shows it.
  async scheduled(controller, env, ctx) {
    ensureBooted(env);
    if (bootError) throw new Error(`Soli failed to boot: ${bootError}`);
    const job = queue
      .then(async () => {
        const bytes = encoder.encode(JSON.stringify({ cron: controller.cron }));
        const ptr = wasm.soli_alloc(bytes.length);
        new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
        const out = await runCron(ptr, bytes.length);
        return JSON.parse(decoder.decode(new Uint8Array(wasm.memory.buffer, out, wasm.soli_response_len())));
      })
      .finally(() => endSql(ctx));
    queue = job.catch(() => {});
    const report = await job;
    const ran = report.ran || [];
    if (ran.length === 0) console.warn(`cron ${controller.cron}: no static cron declares it`);
    for (const r of ran) {
      if (r.status === "done") console.log(`cron ${controller.cron}: ${r.job} done`);
      else console.error(`cron ${controller.cron}: ${r.job} failed: ${r.error}`);
    }
    const failed = ran.filter((r) => r.status !== "done");
    if (failed.length) throw new Error(`${failed.map((r) => r.job).join(", ")} failed`);
  },

  // Jobs enqueued on the edge (`perform_later`, `Job.enqueue*`) arrive here as
  // Cloudflare Queue messages. Each runs in turn, behind any request inside
  // wasm, and is acknowledged, retried after the job's backoff, or — its
  // retry budget spent — acknowledged and logged as dead.
  async queue(batch, env, ctx) {
    ensureBooted(env);
    if (bootError) {
      console.error(`Soli failed to boot: ${bootError}`);
      batch.retryAll();
      return;
    }
    for (const message of batch.messages) {
      const job = queue.then(() => dispatchJob(message)).finally(() => endSql(ctx));
      queue = job.catch(() => {});
      let report;
      try {
        report = await job;
      } catch (error) {
        report = { status: "retry", delay: 30, error: String(error) };
      }
      const name = report.job ? `${report.job.handler} ${report.job.id}` : "job";
      if (report.status === "done") {
        message.ack();
      } else if (report.status === "retry") {
        console.error(`${name} failed (attempt ${message.attempts}), retrying in ${report.delay}s: ${report.error}`);
        message.retry({ delaySeconds: report.delay });
      } else {
        console.error(`${name} is dead after ${message.attempts} attempts: ${report.error}`);
        message.ack();
      }
    }
  },
};
