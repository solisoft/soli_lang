// Runs the Worker's own `liveSocket` (cut out of edge/js/worker.js, with the
// Durable Object and the wasm frame call stubbed) against upgrade requests
// from the app's origin and from elsewhere. Prints one line per case.
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../../edge/js/worker.js", import.meta.url), "utf8");
const start = source.indexOf("async function liveSocket(");
let depth = 0;
let end = -1;
for (let i = source.indexOf("{", start); i < source.length; i++) {
  if (source[i] === "{") depth++;
  if (source[i] === "}" && --depth === 0) {
    end = i + 1;
    break;
  }
}
if (start < 0 || end < 0) throw new Error("liveSocket not found in worker.js");

const liveFrame = async () => ({ id: "instance" });
const liveSocket = new Function("liveFrame", `${source.slice(start, end)}; return liveSocket;`)(liveFrame);

let forwarded = null;
const env = (appHosts) => ({
  SOLI_APP_HOSTS: appHosts,
  SOLI_LIVE: {
    idFromName: (name) => name,
    get: () => ({
      fetch: async (request) => {
        forwarded = request;
        return new Response("upgraded", { status: 200 });
      },
    }),
  },
});

async function upgrade(headers, appHosts = "") {
  forwarded = null;
  const url = new URL("https://app.example.com/live/socket/counter");
  const request = new Request(url, { headers: { upgrade: "websocket", ...headers } });
  const response = await liveSocket(request, env(appHosts), {}, url);
  const room = forwarded ? forwarded.headers.get("x-soli-live-room") : "-";
  return `${response.status} room=${room}`;
}

const cases = [
  ["same origin", { origin: "https://app.example.com" }],
  ["another site", { origin: "https://evil.example" }],
  ["a look-alike host", { origin: "https://app.example.com.evil.example" }],
  ["a malformed origin", { origin: "not a url" }],
  ["a declared second host", { origin: "https://www.example.com" }, "www.example.com, other.example.com"],
  ["no origin (not a browser)", {}],
  ["a client-written room header", { origin: "https://app.example.com", "x-soli-live-room": "someone-elses" }],
];
for (const [name, headers, appHosts] of cases) {
  console.log(`${name}: ${await upgrade(headers, appHosts)}`);
}
