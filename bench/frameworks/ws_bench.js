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

// ActionCable is not raw WebSocket: it speaks a JSON subprotocol over /cable.
// A client must wait for {"type":"welcome"}, subscribe to a channel, wait for
// {"type":"confirm_subscription"}, and only then exchange messages — and the
// server's own pings and acks arrive on the same socket. PROTOCOL=actioncable
// makes the harness do all of that, so Rails is measured on the same workload
// as the raw-socket stacks rather than on a handshake it does not have.
const PROTOCOL = process.env.PROTOCOL || 'raw';
const CHANNEL = process.env.CHANNEL || 'EchoChannel';
const ACTION = process.env.ACTION || 'echo';
const AC_ID = JSON.stringify({ channel: CHANNEL });

function isCable() { return PROTOCOL === 'actioncable'; }

/** Open one socket, resolved when it is ready to carry benchmark messages. */
function openSocket(onPayload) {
  return new Promise((resolve, reject) => {
    const ws = isCable()
      ? new WebSocket(url, ['actioncable-v1-json'], { perMessageDeflate: false })
      : new WebSocket(url, { perMessageDeflate: false });
    let ready = false;
    ws.on('error', () => { if (!ready) reject(new Error('connect failed')); });
    if (!isCable()) {
      ws.on('open', () => { ready = true; resolve(ws); });
      if (onPayload) ws.on('message', onPayload);
      return;
    }
    ws.on('message', (raw) => {
      let msg;
      try { msg = JSON.parse(raw.toString()); } catch { return; }
      if (msg.type === 'welcome') {
        ws.send(JSON.stringify({ command: 'subscribe', identifier: AC_ID }));
      } else if (msg.type === 'confirm_subscription') {
        ready = true; resolve(ws);
      } else if (msg.message !== undefined && onPayload) {
        // Only real channel payloads count — pings and acks are not deliveries.
        onPayload(msg.message);
      }
    });
  });
}

/** Send one benchmark message on an already-ready socket. */
function sendPayload(ws) {
  if (!isCable()) return ws.send('x');
  ws.send(JSON.stringify({ command: 'message', identifier: AC_ID, data: JSON.stringify({ action: ACTION, body: 'x' }) }));
}

/** Open `n` sockets, resolving once they are all open (or one fails). */
async function connectAll(n, onMessage) {
  const results = await Promise.allSettled(
    Array.from({ length: n }, () => openSocket(onMessage))
  );
  const sockets = results.filter((r) => r.status === 'fulfilled').map((r) => r.value);
  return { sockets, opened: sockets.length, failed: n - sockets.length };
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

  const onReply = (ws, i) => {
    received++;
    const t = pending.get(i);
    if (t !== undefined) { latencies.push(Number(process.hrtime.bigint() - t) / 1e6); pending.delete(i); }
    if (running) { pending.set(i, process.hrtime.bigint()); sendPayload(ws); sent++; }
  };
  sockets.forEach((ws, i) => {
    if (ws.readyState !== WebSocket.OPEN) return;
    if (isCable()) {
      ws.on('message', (raw) => {
        let msg; try { msg = JSON.parse(raw.toString()); } catch { return; }
        if (msg.message !== undefined) onReply(ws, i);
      });
    } else {
      ws.on('message', () => onReply(ws, i));
    }
  });
  // Prime one in-flight message per socket, so each connection keeps exactly
  // one request outstanding — the socket equivalent of concurrency = conns.
  sockets.forEach((ws, i) => {
    if (ws.readyState !== WebSocket.OPEN) return;
    pending.set(i, process.hrtime.bigint()); sendPayload(ws); sent++;
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
    sendPayload(pub); publishes++;
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
