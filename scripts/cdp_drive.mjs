// Drive a headless Chromium over the DevTools protocol in REAL time.
//
// `chromium --screenshot --virtual-time-budget` cannot be used to verify pages
// that rely on Web Workers (PDF.js is one): virtual time does not advance the
// worker's event loop, so the page is captured mid-flight and every result
// looks like a hang. This drives a real browser instead, so waits are genuine
// and synthetic mouse input can be dispatched.
//
//   node scripts/cdp_drive.mjs <script.json> [--port 9333]
//
// The step file is a JSON array; supported steps:
//   {"goto": url}
//   {"waitFor": "<js expression>", "timeout": ms}   – poll until truthy
//   {"eval": "<js expression>", "as": "name"}       – record the result
//   {"mouse": [[x,y],…]} | {"mousePath": "<js → [[x,y],…]>"}  – press, move…, release
//   {"key": "Delete"}
//   {"shot": "path.png"}
//   {"sleep": ms}
//
// Prints a JSON object of everything recorded by `as`, plus any page errors.
import { setTimeout as sleep } from "node:timers/promises";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire("file:///" + process.cwd() + "/www/");
const WebSocket = require("ws");

const args = process.argv.slice(2);
const stepFile = args[0];
const portFlag = args.indexOf("--port");
const port = portFlag >= 0 ? Number(args[portFlag + 1]) : 9333;
const steps = JSON.parse(readFileSync(stepFile, "utf8"));

// Connect to a browser the caller already started, e.g.
//   chromium --headless=new --no-sandbox --remote-debugging-port=9333 \
//            --user-data-dir=$HOME/soli-shots/cdp-profile about:blank &
// Spawning it from here proved flaky under snap confinement (the launcher can
// hand off to an existing process and exit before the port is up), and reusing
// one browser across runs is faster anyway.
const chrome = { kill() {} };

const bail = async (msg, code = 1) => {
  console.log(JSON.stringify({ error: msg }, null, 2));
  try { chrome.kill("SIGKILL"); } catch {}
  process.exit(code);
};

// Wait for the DevTools endpoint to come up.
let target = null;
for (let i = 0; i < 120; i++) {
  try {
    const r = await fetch("http://127.0.0.1:" + port + "/json/list");
    const list = await r.json();
    target = list.find((t) => t.type === "page" && !t.url.startsWith("chrome-extension:"));
    if (target) break;
  } catch {}
  await sleep(250);
}
if (!target) await bail("devtools endpoint never appeared");

const ws = new WebSocket(target.webSocketDebuggerUrl, { perMessageDeflate: false });
await new Promise((res, rej) => { ws.once("open", res); ws.once("error", rej); });

let msgId = 0;
const pending = new Map();
const pageErrors = [];
const consoleErrors = [];

ws.on("message", (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.id && pending.has(m.id)) {
    const { resolve, reject } = pending.get(m.id);
    pending.delete(m.id);
    m.error ? reject(new Error(JSON.stringify(m.error))) : resolve(m.result);
    return;
  }
  if (m.method === "Runtime.exceptionThrown") {
    const d = m.params.exceptionDetails;
    pageErrors.push(d.exception?.description || d.text);
  }
  if (m.method === "Runtime.consoleAPICalled" && m.params.type === "error") {
    consoleErrors.push(m.params.args.map((a) => a.value ?? a.description).join(" "));
  }
});

const send = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const id = ++msgId;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });

await send("Page.enable");
await send("Runtime.enable");

const evaluate = async (expr) => {
  const r = await send("Runtime.evaluate", {
    expression: expr,
    returnByValue: true,
    awaitPromise: true,
  });
  if (r.exceptionDetails) {
    throw new Error(r.exceptionDetails.exception?.description || "eval failed");
  }
  return r.result.value;
};

const mouse = async (type, x, y, button = "left") =>
  send("Input.dispatchMouseEvent", {
    type, x, y, button,
    buttons: type === "mouseReleased" ? 0 : 1,
    clickCount: 1,
  });

const recorded = {};

try {
  for (const step of steps) {
    if (step.goto) {
      await send("Page.navigate", { url: step.goto });
      await sleep(400);
    } else if (step.waitFor) {
      const limit = step.timeout ?? 20000;
      const t0 = Date.now();
      let ok = false;
      while (Date.now() - t0 < limit) {
        try { if (await evaluate(step.waitFor)) { ok = true; break; } } catch {}
        await sleep(150);
      }
      if (!ok) throw new Error("waitFor timed out: " + step.waitFor);
    } else if (step.eval) {
      const v = await evaluate(step.eval);
      if (step.as) recorded[step.as] = v;
    } else if (step.mouse || step.mousePath) {
      // `mousePath` is a JS expression evaluated in the page that returns
      // [[x,y],…] in viewport coordinates — the only practical way to aim at an
      // element whose position depends on layout.
      const pts = step.mousePath ? await evaluate(step.mousePath) : step.mouse;
      await mouse("mousePressed", pts[0][0], pts[0][1], step.button || "left");
      for (const [x, y] of pts.slice(1)) { await mouse("mouseMoved", x, y, step.button || "left"); await sleep(30); }
      const last = pts[pts.length - 1];
      await mouse("mouseReleased", last[0], last[1], step.button || "left");
      await sleep(120);
    } else if (step.key) {
      await send("Input.dispatchKeyEvent", { type: "keyDown", key: step.key, code: step.key });
      await send("Input.dispatchKeyEvent", { type: "keyUp", key: step.key, code: step.key });
      await sleep(80);
    } else if (step.shot) {
      const r = await send("Page.captureScreenshot", { format: "png" });
      const { writeFileSync } = await import("node:fs");
      writeFileSync(step.shot, Buffer.from(r.data, "base64"));
    } else if (step.sleep) {
      await sleep(step.sleep);
    }
  }
  console.log(JSON.stringify({ ok: true, recorded, pageErrors, consoleErrors }, null, 2));
} catch (e) {
  console.log(JSON.stringify({ ok: false, error: String(e), recorded, pageErrors, consoleErrors }, null, 2));
} finally {
  try { ws.close(); } catch {}
  try { chrome.kill("SIGKILL"); } catch {}
}
