# Event Streaming from Soli with `es` — a Kafka-Shaped Broker You Can Actually Run

*Updated October 2026: Soli now ships a driver for `es`, the `ES` class, and this post uses it. It replaces the hand-written HTTP wrapper of the first version, and it speaks `es`'s binary protocol when the broker offers it.*

Most "event streaming" guides start by installing Java, then Kafka, then ZooKeeper (or KRaft, or Redpanda, or…), then a four-broker `docker-compose.yml`, then a schema registry. Forty minutes in, you still haven't published an event. For an app that just needs a durable, ordered log between two services — say, an order-created stream that a billing worker drains overnight — that's an enormous tax.

`es` is the answer to "what's the smallest thing with the *shape* of Kafka?" It's a single Rust binary that persists records to disk, supports topics with partitions, tracks consumer-group offsets server-side, and dedupes retries from idempotent producers. Everything is an HTTP+JSON call; produce and consume are also available over a small binary protocol for when throughput matters. There's no ZooKeeper, no KRaft, no protocol buffers.

This post wires `es` to a Soli app end-to-end: start the broker, point the built-in `ES` class at it, emit events from a controller, make retries safe, and drain them with a background job that uses a consumer group so it can resume after a restart.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/event-streaming-es.jpg" width="1024" height="576" alt="Architecture of event streaming with Soli and the lightweight es broker: controller produces events via HTTP, es stores them durably with partitions, background job consumes using consumer groups with offset tracking." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Soli + `es`: simple HTTP production, durable partitioned log, and resumable consumption via background jobs — with almost no operational overhead.</figcaption>
</figure>

## What `es` Looks Like on the Wire

Before any Soli code, the conceptual map. `es` exposes a small HTTP surface — small enough to fit in one table, with three logical groupings:

| Method + Path | Purpose |
|---|---|
| `GET  /healthz` | Liveness probe |
| `GET  /metrics` | Prometheus text-format metrics (size, lag, retention/compaction counters) |
| `GET  /topics` | List topics |
| `POST /topics` | Create a topic with `partitions: N` and optional config patch |
| `GET  /topics/:name` | Describe (offsets, size, config per partition) |
| `GET  /topics/:name/config` | Read resolved topic config |
| `PUT  /topics/:name/config` | Patch topic config (retention, cleanup_policy, segment_bytes) |
| `POST /topics/:name/produce` | Append records |
| `GET  /topics/:name/consume?partition=…&offset=…` | Read records at a position |
| `GET  /groups/:group/consume?topic=…&partition=…` | Read from a group's committed offset |
| `POST /groups/:group/commit` | Advance the group's offset |
| `GET  /groups/:group/offsets` | Inspect what a group has committed |
| `POST /groups/:group/join` | Join a group with a topic subscription; receive assigned partitions |
| `POST /groups/:group/heartbeat` | Keep membership alive; learn when a rebalance is required |
| `POST /groups/:group/leave` | Cleanly drop out of a group (triggers immediate rebalance) |
| `GET  /groups/:group/assignment?member_id=…` | Re-fetch the current assignment for a member |
| `POST /admin/run-retention` | Force a synchronous retention pass (useful in tests) |
| `POST /admin/run-compaction` | Force a synchronous compaction pass |
| `GET  /admin/keys` | List API keys (requires `admin` ACL) |
| `POST /admin/keys` | Mint a new API key with ACLs + rate limits |
| `DELETE /admin/keys/:key_id` | Revoke a key (soft-disable) |
| `GET  /admin/producers` | List idempotent-producer state (last seen seq + offset per partition) |
| `DELETE /admin/producers/:producer_id` | Forget a producer's dedup state |
| `POST /admin/reset-offsets` | Rewind every group on a topic back to its start_offset |

A *record* is `{key, value, partition?}`. The key is what routes the record to a partition (consistent-hash); pass `partition` explicitly if you want to override. Within a partition, records are strictly ordered and assigned a monotonic `offset`. That's the whole data model.

With `--bind-binary`, the broker also listens on a second port for a length-prefixed binary protocol that carries produce, consume and ping — raw bytes, no JSON, one long-lived connection per client. Everything else in the table stays HTTP-only.

## Step 1: Boot the Broker

`es` is a workspace with two binaries: `es-broker` (the server) and `es` (the CLI). Once built, the broker is one command:

```bash
$ es-broker --bind 127.0.0.1:9000 --bind-binary 127.0.0.1:9001 --data-dir ./data
INFO broker listening addr=127.0.0.1:9000
```

Then create the topic we'll use throughout the post. Three partitions is enough to demonstrate keying without making the example noisy:

```bash
$ es topic create --name orders --partitions 3
created topic 'orders' with 3 partition(s)

$ es topic describe --name orders
topic orders
  config:
    cleanup_policy         = delete
    segment_bytes          = 67108864
    tombstone_retention_ms = 86400000
    retention_ms           = (unset, infinite)
    retention_bytes        = (unset, infinite)
  partition 0  start=     0  end=     0  segments=1  size=0B
  partition 1  start=     0  end=     0  segments=1  size=0B
  partition 2  start=     0  end=     0  segments=1  size=0B
```

`./data/topics/orders/` now holds an append-only log per partition. A redeploy of your Soli app doesn't touch it. A crash of `es-broker` doesn't lose acknowledged records — they're fsynced segments on disk.

That last sentence is true at the default `--flush-every-records 1`, which fsyncs after every append. If a benchmark shows you're disk-fsync-bound (typical past a few thousand records/sec on a single partition), the flag lets you batch — `--flush-every-records 100` fsyncs once every hundred appends, trading up to ~99 un-fsynced records on a hard crash for an order-of-magnitude throughput bump. Records are still durable in the OS page cache the instant they're acked, so the loss window only opens on a kernel panic or power cut, not on `es-broker` segfaulting. Pick `1` for ledger-style data where every record matters; pick higher for telemetry where the producer can replay a small tail.

By default a topic is `cleanup_policy = delete` with **infinite retention** — the log grows forever. For a stream where consumers always catch up within a known window (most operational event streams), set a retention bound at create time so disk doesn't surprise you in production:

```bash
# Keep at most 7 days of orders, or 10 GiB, whichever bites first.
$ es topic create --name orders --partitions 3 \
    --retention-ms 604800000 \
    --retention-bytes 10737418240
```

You can change either bound later — `es topic alter --name orders --retention-ms ...` issues a `PUT /topics/orders/config` and a background reaper deletes segments older than the cutoff on its next pass.

Two env vars tell the Soli app where the broker is — the HTTP API, and the binary listener:

```bash
# .env
ES_BROKER=http://127.0.0.1:9000
ES_BINARY=127.0.0.1:9001
```

`ES_BROKER` is the same variable the `es` CLI reads. Leave `ES_BINARY` out and everything goes over HTTP; it works the same, a little slower.

### Locking It Down with API Keys

By default `es-broker` accepts every request — perfect for `localhost` development, very wrong for production. Flip the broker into authenticated mode with one flag:

```bash
$ es-broker --auth required --data-dir ./data
WARN auth required + empty store: generated bootstrap admin key (secret written to file, rotate after first use) bootstrap_key_path="./data/bootstrap.key"
INFO broker listening addr=127.0.0.1:9000 scheme=http
```

The first time you start with `--auth required` on an empty store, `es` generates a bootstrap admin key and writes the secret to `./data/bootstrap.key` (mode 0600). Read it once, use it to mint the keys you actually want, then delete the file:

```bash
$ export ES_AUTH=$(cat ./data/bootstrap.key)

# A write-only key for the orders service, scoped to the "orders" topic prefix.
$ es key create --name orders-writer --acl write:orders
key_id: key_a3f12c
name:   orders-writer
acl:    write:orders

SECRET (shown once, save it now):
  esk_xK9pQ2vL...

# A read-only key for the billing drain.
$ es key create --name billing-reader --acl read:orders
key_id: key_b7d40e
name:   billing-reader
acl:    read:orders

SECRET (shown once, save it now):
  esk_mR3jH8cN...
```

The ACL grammar is `action:topic_prefix` — `read`, `write`, or `admin`, with `*` matching every topic. `write:` implies `read:` on the same prefix, so the writer can verify what it just produced; `read:` does not imply `write:`. Add `--produce-bytes-per-sec 1048576` (or `--consume-bytes-per-sec`) when minting the key to cap a noisy neighbor — the broker enforces the budget per request and replies with `429 Too Many Requests` plus a retry hint if the bucket runs dry.

Now the Soli app needs two env vars, one per service:

```bash
# Producer (controllers): .env on the web tier
ES_BROKER=http://127.0.0.1:9000
ES_BINARY=127.0.0.1:9001
ES_AUTH=esk_xK9pQ2vL...

# Consumer (drain): .env on the worker tier
ES_BROKER=http://127.0.0.1:9000
ES_BINARY=127.0.0.1:9001
ES_AUTH=esk_mR3jH8cN...
```

For a real deployment, also pass `--tls-cert` and `--tls-key` to the broker so the secret doesn't travel over the wire in clear. `es-broker` speaks rustls directly, on both ports; no nginx in front needed. Point `ES_BROKER` at `https://…` and the driver uses TLS on the binary port too — `ES_CA_FILE` names your CA if the certificate is not from a public one.

## Step 2: The `ES` Class

There is nothing to write here any more. `ES` is built into Soli, configured by those env vars, and available in every controller, job, script and view helper:

```soli
print(ES.config()["transport"])   # binary — or http, without ES_BINARY
ES.ping                           # true, or raises if the broker is down
ES.create_topic("orders", {"partitions": 3, "retention_ms": 604800000})
```

What it does for you, that the first version of this post did by hand:

- **Transport.** `ES.emit`, `ES.produce` and `ES.consume` go over the binary protocol when `ES_BINARY` is set, on connections pooled per application and checked before reuse; topics, groups and commits go over HTTP, the only place they exist.
- **Serialization.** A value that is not a String is sent as JSON. It comes back as the String it was; `JSON.parse` it.
- **Auth.** `ES_AUTH` rides along on both protocols. A refused token raises `ES.emit: the token was refused — check ES_AUTH`, a missing grant `… forbidden — the key lacks a grant`: no retry fixes those, and the message says so.
- **Errors.** Every failure raises, naming the call: `ES.consume: topic 'orders' not found (404)`. Unknown option keys raise too, so a typo in `"max_record"` is not silently ignored.

The full reference is on the [`ES` page](/docs/builtins/es).

## Step 3: Produce from a Controller

A signup flow that emits an `order.created` event after the row is persisted. The user gets their redirect; the billing worker, the analytics pipeline, the welcome-email job — all the downstream consumers wake up on their own schedule.

```soli
# app/controllers/orders_controller.sl
class OrdersController < Controller
  # POST /orders — persist the order, then announce it
  def create(req)
    @order = Order.create(permit(params, {"user_id": true, "total": true, "currency": true}))
    return render("orders/new", {}, {"status": 422}) if @order._errors

    # Keyed by user: every order of one user lands in the same partition,
    # which is how es (and Kafka) keep per-user ordering.
    ES.emit("orders", @order.user_id.to_s, {
      "event": "order.created",
      "order_id": @order._key,
      "user_id": @order.user_id,
      "total": @order.total,
      "currency": @order.currency,
      "created_at": DateTime.utc.to_iso
    })
    redirect("/orders/#{@order._key}")
  end
end
```

Two design notes:

1. **The event is `order.created`, not `Charge stripe and email the user`.** Producers describe *what happened*; consumers decide *what to do*. That's what makes this scalable later — a new consumer (fraud scoring, say) joins the topic without the controller knowing it exists.
2. **The key is `user_id`, not `order.id`.** All of a single user's orders are processed in submission order by a single consumer. Different users may interleave, which is exactly what you want for throughput.

If the broker is briefly down, `ES.emit` raises, the controller returns a 500, and the user retries. There's no in-memory queue eating the event. That's a deliberate tradeoff: at the cost of one more hop in the happy path, you get a failure story the user understands. Need fire-and-forget? Emit from a job (`OrderEventJob.perform_later({...})`) — but most apps don't.

### Making Retries Safe with `producer_id`

The naive retry above has one subtle hole: what if the broker *did* append the record and only the reply was lost? The caller retries, and now there are two `order.created` events for the same order. The billing drain charges Stripe twice. (The driver never retries a produce on its own, for exactly this reason.)

`es` closes the gap with idempotent producers. Tag records with a `producer_id` and a `sequence`, and the broker remembers, per producer **and partition**, the last sequence it appended. The rules are strict, and worth knowing before writing any code:

- The next record must carry exactly **last + 1**. A jump is refused as a *sequence gap*; anything already seen comes back with its original offset and `"duplicate": true` instead of being appended again.
- The first record a producer sends to a partition may start anywhere.
- The broker keeps that state across restarts, for seven days after a producer was last seen.

So a sequence is a counter kept by **one sender**, per partition — not a database id, which has gaps and is shared by every process. The shape that fits is a sender that owns its producer id for the length of one run, fixes each record's sequence *before* the first attempt, and retries with the same one:

```soli
# app/services/order_relay.sl
class OrderRelay
  producer_id: String
  sequences: Hash

  new()
    # A fresh id per run: the broker accepts any first sequence from it.
    @producer_id = "orders-relay-#{DateTime.utc.to_unix}"
    @sequences = {}
  end

  # Send `event` to `partition` once, however many attempts it takes.
  def publish(partition: Int, key: String, event: Hash) -> Hash
    sequence = (@sequences[partition] ?? -1) + 1
    options = {"producer_id": @producer_id, "sequence": sequence, "partition": partition}
    attempt = 1
    result = nil
    while result.nil?
      try
        result = ES.emit("orders", key, event, options)
      catch error
        throw error if attempt == 3

        sleep(0.2 * attempt)
        attempt += 1
      end
    end
    @sequences[partition] = sequence
    result
  end
end
```

The relay picks the partition itself (`user_id % 3` for a three-partition topic): its counters are per partition, so it has to know where each record goes. A retry that the broker had in fact appended returns `{"duplicate": true, "offset": …}` with the original offset, and the drain side sees one record, not two. One relay per producer id: two processes sharing an id would interleave their counters and trip the gap check. To wipe a producer's state — after retiring it, say — use `es producer revoke --id …`.

If you don't need this, skip it. Reach for `producer_id` when the *consumer* can't tolerate duplicates and the *producer* might retry — typically anything that drives an external side-effect (Stripe, SES, a webhook).

## Step 4: Drain with a Consumer Group

The consumer is the interesting half. We want:

- **Resumable** — restart the worker without re-processing what's already done.
- **Per-partition parallelism** — three partitions, three drains, three times the throughput.
- **At-least-once delivery** — handlers may run twice on a crash; they must be idempotent.

A consumer group on `es` gives us the first two for free; the third is on us.

```soli
# app/jobs/orders_drain_job.sl
class OrdersDrainJob
  static def perform(args: Hash)
    partition = args["partition"]
    drained = 0
    while true
      page = ES.group_consume("billing", "orders", partition, {"max_records": 200})
      break if page["records"].length == 0

      page["records"].each { |record| OrdersDrainJob.handle(JSON.parse(record["value"])) }
      # Commit after the batch is handled: a crash replays it (at-least-once).
      ES.commit("billing", "orders", partition, page["next_offset"])
      drained += page["records"].length
    end
    {"partition": partition, "drained": drained}
  end

  static def handle(event)
    match event["event"] {
      "order.created" => BillingService.charge(event["order_id"]),
      "order.refunded" => BillingService.refund(event["order_id"]),
      _ => nil
    }
  end
end
```

Three subtleties:

- **Commit happens after the handler returns, not before.** If `BillingService.charge` raises mid-batch, the job unwinds without committing, and the next run sees the same records again. That's the at-least-once contract — combined with an idempotency key on the billing side (e.g. `order_id` as Stripe's `Idempotency-Key`), double delivery is harmless.
- **An empty page is the stop signal.** `ES.group_consume` reads from the group's committed offset; once that reaches the high-water mark, there is nothing left.
- **`partition` is a job argument.** One `OrdersDrainJob` per partition runs in parallel. Schedule them with a tiny dispatcher:

```soli
# app/jobs/orders_dispatcher_job.sl
class OrdersDispatcherJob
  static def perform(args: Hash)
    partitions = ES.topic("orders")["partitions"].length
    (0..partitions).each { |partition| OrdersDrainJob.perform_later({"partition": partition}) }
  end
end
```

Schedule the dispatcher every minute — or every five seconds, depending on how fresh you want the billing pipeline to be. A drain that finds nothing costs one request; cheap enough to poll aggressively.

### Letting the Broker Assign Partitions

The dispatcher above is fine when *you* decide which partitions each drain handles. The moment you want to scale by adding worker processes — or survive a worker crashing without one partition's stream falling silent — the static fan-out stops being enough. You'd be reinventing the part of Kafka that does this for you.

`es` does it for you too. Every worker calls `ES.join("billing", ["orders"])`, the broker hands each one a slice of the partitions, and a heartbeat-based failure detector rebalances when a worker dies. The drain stays the same — only the *source* of `partition` changes, from a job argument to whatever the broker assigned on this generation:

```soli
# app/services/orders_worker.sl — run it from a long-running job
class OrdersWorker
  static def run
    member = ES.join("billing", ["orders"])
    beat_at = DateTime.utc.to_unix
    try
      while true
        member["assignment"].each { |tp| OrdersDrainJob.perform({"partition": tp["partition"]}) }

        # Heartbeat every 5 s, well inside the coordinator's 15 s timeout —
        # and the moment to learn that the group is rebalancing.
        if DateTime.utc.to_unix - beat_at >= 5
          status = ES.heartbeat("billing", member["member_id"], member["generation"])["status"]
          beat_at = DateTime.utc.to_unix
          if status == "rebalance_required"
            member = ES.join("billing", ["orders"], member["member_id"])
          elsif status == "unknown_member"
            member = ES.join("billing", ["orders"])     # evicted: start fresh
          end
        end
        sleep(0.2)
      end
    finally
      # Leaving turns a shutdown into an immediate rebalance instead of a
      # 15-second wait for the heartbeat to time out.
      ES.leave("billing", member["member_id"]) rescue nil
    end
  end
end
```

What the coordinator gives you, beyond the static dispatcher:

- **Scaling by adding workers.** Run two `OrdersWorker` processes — each gets ~half the partitions. Run six — each gets one (for a 6-partition topic). No code changes, no redeploy.
- **Failover without manual intervention.** Stop sending heartbeats (worker crashed, network partitioned, a pause too long) and within the member timeout (15 s by default) the coordinator evicts the dead member and reassigns its partitions to the survivors. The replay lands at the survivor's next `ES.group_consume` — at-least-once does the rest.
- **A real shutdown story.** `ES.leave` in a `finally` turns a SIGTERM into an immediate rebalance. Rolling deploys depend on this.

When to keep the static dispatcher: one worker process, a fixed deployment, scheduling through Soli's job runner. When to switch to `ES.join`: whenever you want to scale horizontally or tolerate a worker failing.

## Step 5: Inspect from the CLI

`es` ships with a CLI that talks to the same HTTP API, which means *the operator's view and the app's view are identical*. No "Kafka tools" rabbit hole.

```bash
# What partitions does the topic have, how full are they, and what's the config?
$ es topic describe --name orders
topic orders
  config:
    cleanup_policy         = delete
    segment_bytes          = 67108864
    tombstone_retention_ms = 86400000
    retention_ms           = 604800000
    retention_bytes        = 10737418240
  partition 0  start=     0  end=    47  segments=1  size=14256B
  partition 1  start=     0  end=    52  segments=1  size=15732B
  partition 2  start=     0  end=    49  segments=1  size=14808B

# Where is the billing group sitting? Are we caught up?
$ es group show --name billing
topic orders
  partition 0 -> offset 47
  partition 1 -> offset 52
  partition 2 -> offset 49

# Tail one partition without touching the group's committed offset:
$ es consume --topic orders --partition 0 --offset 40 --max 5
p=0 off=40 ts=1748039210123 key=Some("17") value={"event":"order.created","order_id":98,...}
p=0 off=41 ts=1748039211004 key=Some("23") value={"event":"order.created","order_id":99,...}
…
-- next_offset=45 high_watermark=47 (5 record(s))
```

That third command is the unsung hero of debugging. You can read events from any offset without disturbing the consumer group — perfect for the "what was the payload of that event 200 records ago?" question. The same `/topics/:name/consume` endpoint backs it, which means an admin page in your Soli app can offer it too with a five-line controller action.

For continuous observability, point Prometheus at `/metrics`:

```
$ curl -s http://127.0.0.1:9000/metrics | grep -E 'es_(group_lag|partition_size)' | head
es_partition_size_bytes{topic="orders",partition="0"} 14256
es_partition_size_bytes{topic="orders",partition="1"} 15732
es_partition_size_bytes{topic="orders",partition="2"} 14808
es_group_lag{group="billing",topic="orders",partition="0"} 0
es_group_lag{group="billing",topic="orders",partition="1"} 0
es_group_lag{group="billing",topic="orders",partition="2"} 0
```

`es_group_lag` is the single most useful number to alert on — it's the distance between the high-water mark and the group's committed offset. If billing falls behind by, say, 10k records for more than a minute, the worker is stuck. Wire that into Grafana and you've got the operational view that takes a Kafka deployment a weekend.

If you turned on `--auth required`, `es key list` rounds out the audit story — which keys exist, what they're allowed to do, and which have been revoked:

```bash
$ es key list
key_a3f12c  orders-writer
  acl: write:orders
key_b7d40e  billing-reader
  acl: read:orders
key_c91f30  old-debug-key [revoked]
  acl: admin:*
```

`es key revoke --id key_c91f30` flips the `disabled` flag in `./data/api_keys.json` and the broker rejects further requests carrying that secret with `401`. The secret itself is never stored — only its SHA-256, so a stolen `api_keys.json` doesn't leak credentials, only their hashes.

For coordinator-driven groups, `es group join` and `es group assignment` are the operator's window into who's holding what:

```bash
# Simulate a worker (or use it as a manual consumer in a pinch).
$ es group join --name billing --topic orders
member_id:  m-3f12c8
generation: 4
assignment:
  orders/1
  orders/2

# What does this particular member think it owns right now?
$ es group assignment --name billing --member-id m-3f12c8
generation: 4
assignment:
  orders/1
  orders/2

# Drop the member; the coordinator rebalances within milliseconds.
$ es group leave --name billing --member-id m-3f12c8
```

For the idempotent-producer side, `es producer list` shows what the broker remembers about each known producer — the last sequence it accepted and where it landed on disk. That's the diagnostic you reach for when something looks like it was double-charged or skipped:

```bash
$ es producer list
producer web-orders-1
  topic=orders partition=0 last_seq=4831 last_offset=4831
  topic=orders partition=1 last_seq=4775 last_offset=4775
  topic=orders partition=2 last_seq=4812 last_offset=4812
```

And `es reset-offsets --topic orders` is the "I've cleaned everything up, every consumer group should re-read from the current start" button. Useful after a destructive cleanup (manual segment deletion, a partition rebuild) where the committed offsets no longer make sense. It's a sharp tool — every group on the topic gets rewound, not just one — so it should sit in your runbook, not in any drain code.

## Why This Composition Holds Up

The pieces sit on deliberate boundaries:

- **`ES` doesn't know about orders or billing.** It speaks records. Add a `payments` topic tomorrow and `ES.emit("payments", ...)` works.
- **The controller doesn't know about billing.** It describes the event and is done. A new consumer (analytics, fraud) joins by reading the topic — nobody edits the producer.
- **The drain job doesn't know about the transport.** The same `ES.group_consume` runs over HTTP or the binary protocol, decided by an env var.
- **Auth lives in configuration.** Callers never reference `ES_AUTH` or `Authorization`. Rotating the writer's key is a deploy that changes one env var; no code moves.

That layering is also why the *whole thing* fits in a controller, a job and a service. Kafka's bigness is largely about features you don't have yet — cross-topic transactions, schema registries, geo-replication. `es` punts on most of them. If you ever need them, you migrate. Until then, you ship.

### HTTP or binary?

Both work, and the code does not change. The binary port is the faster one from Soli, because there is no JSON to build or parse and the connection stays open. Measured from one single-threaded Soli script against a broker on a Ryzen 9 9950X (pinned to separate cores, fsync every 1 000 records, median of three runs):

| | binary | HTTP |
|---|---:|---:|
| `ES.emit`, one record per call | 51 200 /s (19.5 µs) | 37 700 /s (26.5 µs) |
| `ES.produce`, 100 records per call | 774 000 records/s | 521 000 records/s |
| `ES.consume`, 1 000 records per page | 1.40 M records/s | 1.15 M records/s |

Two things stand out. Batching beats the transport by an order of magnitude: one `ES.produce` of 100 records costs less than ten `ES.emit` calls. And at the broker's default `--flush-every-records 1`, a single-record emit is bound by the disk, not the wire — 18 600/s over binary, 16 200/s over HTTP. Set `ES_BINARY` when it is there, and batch when you can.

## When to Reach for `es` vs. What You Already Have

Soli has two adjacent tools that overlap with `es`:

- **Background jobs** (see [Background Jobs and Cron](/docs/blog/background-jobs-and-cron)) — durable, retried, scheduled. The difference: a job is a *unit of work to do once*; a topic is a *stream of facts other systems will read repeatedly, in order*. Use jobs for "send this email"; use `es` for "every order ever placed, replayable by any consumer."
- **WebSocket broadcasts** — pushed to connected clients, ephemeral, no replay. Use WS for "tell every open dashboard a new order came in"; use `es` for "let the billing service catch up on the last hour of orders after a deploy."

You'll often want both. A controller can `ES.emit` and `ws_broadcast` in the same request: the event lands in the durable log for the billing drain, *and* the live order board updates in real time. The two are orthogonal.

## What's Next

A few directions to extend this:

- **Dead-letter handling.** A handler that has raised three times in a row probably won't succeed on the fourth. Track per-record attempt counts in a SolidB collection keyed by `(topic, partition, offset)` and route the record to a `dead_letters` topic instead of blocking the drain.
- **Schema validation.** `JSON.parse` is generous about what comes in. Validate the payload's shape with `User.validate(...)` (or a JSON Schema check) inside `handle` and route bad records to a `parse_failures` topic.
- **Compacted topics for current-state streams.** When the stream represents *state* rather than *facts* — `user.profile_updated`, `cart.contents_changed`, `account.balance` — you don't want the full history, you want the latest value per key. Create the topic with `--cleanup-policy compact`, write the current state as the `value` (with `null` value to tombstone a key), and the broker keeps only the most recent record per key forever:

  ```bash
  $ es topic create --name user_profiles --partitions 3 --cleanup-policy compact
  ```

  A late-joining consumer reads from offset 0 and gets a snapshot of every user that has ever existed, in O(distinct keys) records instead of O(history). Combine with `--cleanup-policy compact,delete` if you also want a time bound — e.g. "latest per user, but drop users untouched for 90 days."

The producer is one line. The consumer is fifteen. The broker is one process. That ratio — three small, well-named pieces — is what makes event streaming feel like a tool rather than an architecture decision.
