#!/usr/bin/env node
// WebSocket harness. oha speaks HTTP only, so the three things worth measuring
// over a socket need their own client:
//
//   capacity   how many concurrent connections a stack holds, and what they cost
//   echo       per-message round trip: messages/sec and latency
//   room       fan-out: one client sends, every connection in the room receives
//
// usage: node ws_bench.js <capacity|echo|room> <url> [connections] [seconds]
const WebSocket = require('ws');

const [, , mode, url, connArg, secArg] = process.argv;
const CONNS = Number(connArg || 500);
const SECS = Number(secArg || 15);

if (!mode || !url) {
  console.error('usage: ws_bench.js <capacity|echo|room> <ws://host/path> [connections] [seconds]');
  process.exit(1);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Open `n` sockets, resolving once they are all open (or one fails). */
async function connectAll(n, onMessage) {
  const sockets = [];
  let opened = 0, failed = 0;
  await new Promise((resolve) => {
    for (let i = 0; i < n; i++) {
      const ws = new WebSocket(url, { perMessageDeflate: false });
      ws.on('open', () => { if (++opened + failed === n) resolve(); });
      ws.on('error', () => { if (opened + ++failed === n) resolve(); });
      if (onMessage) ws.on('message', onMessage);
      sockets.push(ws);
    }
  });
  return { sockets, opened, failed };
}

function percentile(sorted, p) {
  if (!sorted.length) return 0;
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))];
}

async function capacity() {
  const t0 = Date.now();
  const { sockets, opened, failed } = await connectAll(CONNS);
  const elapsed = (Date.now() - t0) / 1000;
  // Hold them a moment so the server's steady-state memory can be sampled.
  await sleep(3000);
  console.log(JSON.stringify({
    mode: 'capacity', requested: CONNS, opened, failed,
    connect_secs: Number(elapsed.toFixed(2)),
    connects_per_sec: Math.round(opened / elapsed),
  }));
  sockets.forEach((s) => s.close());
}

async function echo() {
  const { sockets, opened, failed } = await connectAll(CONNS);
  if (!opened) { console.error('no connections opened'); process.exit(1); }

  const latencies = [];
  let sent = 0, received = 0, running = true;
  const pending = new Map();

  sockets.forEach((ws, i) => {
    if (ws.readyState !== WebSocket.OPEN) return;
    ws.on('message', () => {
      received++;
      const t = pending.get(i);
      if (t !== undefined) { latencies.push(Number(process.hrtime.bigint() - t) / 1e6); pending.delete(i); }
      if (running) { pending.set(i, process.hrtime.bigint()); ws.send('x'); sent++; }
    });
  });
  // Prime one in-flight message per socket, so each connection keeps exactly
  // one request outstanding — the socket equivalent of concurrency = conns.
  sockets.forEach((ws, i) => {
    if (ws.readyState !== WebSocket.OPEN) return;
    pending.set(i, process.hrtime.bigint()); ws.send('x'); sent++;
  });

  const t0 = Date.now();
  await sleep(SECS * 1000);
  running = false;
  const elapsed = (Date.now() - t0) / 1000;
  latencies.sort((a, b) => a - b);
  console.log(JSON.stringify({
    mode: 'echo', connections: opened, failed, seconds: Number(elapsed.toFixed(1)),
    messages: received, msgs_per_sec: Math.round(received / elapsed),
    p50_ms: Number(percentile(latencies, 0.5).toFixed(3)),
    p99_ms: Number(percentile(latencies, 0.99).toFixed(3)),
  }));
  sockets.forEach((s) => s.close());
}

async function room() {
  let delivered = 0;
  const { sockets, opened, failed } = await connectAll(CONNS, () => { delivered++; });
  if (!opened) { console.error('no connections opened'); process.exit(1); }
  await sleep(500);

  // One publisher, everyone else listening: each send should reach `opened`
  // sockets, so delivered/publish tells us whether fan-out is real.
  //
  // The publisher is RATE-LIMITED on purpose. Pumping flat out just outruns the
  // server: publishes are counted client-side and cost nothing, while each one
  // costs the server `opened` sends, so the ratio collapses and measures the
  // client's send loop instead of the fan-out. At a fixed rate the ratio is
  // meaningful — it should equal the number of connections in the room, and
  // anything less means the broadcast is not reaching everyone.
  const RATE = Number(process.env.PUBLISH_RATE || 50); // publishes/sec
  const pub = sockets.find((s) => s.readyState === WebSocket.OPEN);
  delivered = 0;
  let publishes = 0, running = true;
  const t0 = Date.now();
  const timer = setInterval(() => {
    if (!running) return;
    pub.send('x'); publishes++;
  }, Math.max(1, Math.round(1000 / RATE)));
  await sleep(SECS * 1000);
  running = false;
  clearInterval(timer);
  await sleep(1000);
  const elapsed = (Date.now() - t0) / 1000;
  console.log(JSON.stringify({
    mode: 'room', connections: opened, failed, seconds: Number(elapsed.toFixed(1)),
    publishes, delivered,
    fanout_per_publish: Number((delivered / Math.max(publishes, 1)).toFixed(1)),
    deliveries_per_sec: Math.round(delivered / elapsed),
  }));
  sockets.forEach((s) => s.close());
}

const run = { capacity, echo, room }[mode];
if (!run) { console.error(`unknown mode: ${mode}`); process.exit(1); }
run().then(() => process.exit(0));
