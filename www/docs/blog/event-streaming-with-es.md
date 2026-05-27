# Event Streaming from Soli with `es` — a Kafka-Shaped Broker You Can Actually Run

Most "event streaming" guides start by installing Java, then Kafka, then ZooKeeper (or KRaft, or Redpanda, or…), then a four-broker `docker-compose.yml`, then a schema registry. Forty minutes in, you still haven't published an event. For an app that just needs a durable, ordered log between two services — say, an order-created stream that a billing worker drains overnight — that's an enormous tax.

`es` is the answer to "what's the smallest thing with the *shape* of Kafka?" It's a single Rust binary that speaks plain HTTP+JSON, persists records to disk, supports topics with partitions, and tracks consumer-group offsets server-side. There's no Zookeeper, no KRaft, no protocol buffers, no client SDK to keep in lockstep. You produce with a POST. You consume with a GET. You commit an offset with another POST. That's the whole API.

This post wires `es` to a Soli app end-to-end: start the broker, create a topic, build a thin `Es` wrapper, emit events from a controller, and drain them with a background job that uses a consumer group so it can resume after a restart.

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

## Step 1: Boot the Broker

`es` is a workspace with two binaries: `es-broker` (the server) and `es` (the CLI). Once built, the broker is one command:

```bash
$ es-broker --bind 127.0.0.1:9000 --data-dir ./data
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

Set an env var so the Soli app knows where to find the broker:

```bash
# .env
ES_BROKER=http://127.0.0.1:9000
```

### Locking It Down with API Keys

By default `es-broker` accepts every request — perfect for `localhost` development, very wrong for production. Flip the broker into authenticated mode with one flag:

```bash
$ es-broker --auth required --data-dir ./data
WARN auth required + empty store: generated bootstrap admin key (secret written to file, rotate after first use) bootstrap_key_path="./data/keys/bootstrap.key"
INFO broker listening addr=127.0.0.1:9000 scheme=http
```

The first time you start with `--auth required` on an empty store, `es` generates a bootstrap admin key and writes the secret to `./data/keys/bootstrap.key` (mode 0600). Read it once, use it to mint the keys you actually want, then delete the file:

```bash
$ export ES_AUTH=$(cat ./data/keys/bootstrap.key)

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
ES_AUTH=esk_xK9pQ2vL...

# Consumer (drain): .env on the worker tier
ES_BROKER=http://127.0.0.1:9000
ES_AUTH=esk_mR3jH8cN...
```

For a real deployment, also pass `--tls-cert` and `--tls-key` to the broker so the secret doesn't travel over the wire in clear. `es-broker` speaks rustls directly; no nginx in front needed.

## Step 2: A Tiny `Es` Wrapper in Soli

Drop a wrapper in `app/services/` so every controller, job, and script can produce and consume without repeating URL strings. Autoloading means no `import` line at the callsite.

```soli
# app/services/es.sl
class Es
  static def base
    getenv("ES_BROKER") || "http://127.0.0.1:9000"
  end

  # Build the auth headers. Returns an empty hash when ES_AUTH is unset, which
  # lets the same wrapper work against a `--auth disabled` broker in tests.
  static def headers
    token = getenv("ES_AUTH")
    return {} if token.blank?
    {"Authorization": "Bearer #{token}"}
  end

  # Append one or more records to a topic.
  # `records` is an array of {"key": ..., "value": ..., "partition": ..., "sequence": ...}.
  # Pass `producer_id` to opt into idempotent semantics — see "Making Retries Safe" below.
  static def produce(topic, records, producer_id = nil)
    url = "#{base}/topics/#{topic}/produce"
    body = {"records": records}
    body["producer_id"] = producer_id unless producer_id.blank?
    response = HTTP.post_json(url, body, {"headers": headers})
    check!(response, "produce", topic)
    response["body"].to_h
  end

  # Convenience: one record, value is serialized for us.
  # When `producer_id` is set, `sequence` must be a monotonically increasing
  # integer per `(producer_id, topic, partition)`.
  static def emit(topic, key, value, producer_id = nil, sequence = nil)
    record = {"key": key, "value": value.to_json}
    record["sequence"] = sequence unless sequence.nil?
    produce(topic, [record], producer_id)
  end

  # Read from a consumer group's committed position. Returns
  # {"records": [...], "next_offset": N, "high_watermark": N}.
  static def consume(group, topic, partition, max = 100)
    url = "#{base}/groups/#{group}/consume?topic=#{topic}&partition=#{partition}&max_records=#{max}"
    response = HTTP.get(url, headers)
    check!(response, "consume", topic)
    response["body"].to_h
  end

  # Advance the group's committed offset for this partition.
  static def commit(group, topic, partition, offset)
    url = "#{base}/groups/#{group}/commit"
    response = HTTP.post_json(url, {
      "topic": topic,
      "partition": partition,
      "offset": offset
    }, {"headers": headers})
    raise("es commit failed: #{response["status"]} #{response["body"]}") unless response["status"] == 204
    true
  end

  # Number of partitions on a topic — useful for fanning out a drain loop.
  static def partition_count(topic)
    response = HTTP.get("#{base}/topics/#{topic}", headers)
    check!(response, "describe", topic)
    response["body"].to_h["partitions"].length
  end

  # --- consumer-group coordinator ---

  # Join a group. Returns {"member_id": …, "generation": N, "assignment": [{topic, partition}, …]}.
  # Pass nil for `member_id` to let the coordinator pick one on first join.
  static def join_group(group, topics, member_id = nil)
    body = {"topics": topics}
    body["member_id"] = member_id unless member_id.blank?
    response = HTTP.post_json("#{base}/groups/#{group}/join", body, {"headers": headers})
    check!(response, "join", group)
    response["body"].to_h
  end

  # Heartbeat. Returns {"status": "ok"|"rebalance_required"|"unknown_member", "generation"|"current_generation": N}.
  # "rebalance_required" means: stop processing, re-join, get the new assignment.
  # "unknown_member" means: the coordinator evicted us (probably we were too slow); re-join.
  static def heartbeat(group, member_id, generation)
    response = HTTP.post_json("#{base}/groups/#{group}/heartbeat",
      {"member_id": member_id, "generation": generation},
      {"headers": headers})
    check!(response, "heartbeat", group)
    response["body"].to_h
  end

  # Clean shutdown: drop out of the group so the coordinator rebalances immediately
  # instead of waiting for our heartbeat to time out.
  static def leave_group(group, member_id)
    response = HTTP.post_json("#{base}/groups/#{group}/leave",
      {"member_id": member_id}, {"headers": headers})
    raise("es leave failed: #{response["status"]} #{response["body"]}") unless response["status"] == 204
    true
  end

  # Translate the broker's response codes into intelligible exceptions.
  # 401/403 means the ES_AUTH token is wrong or under-privileged — those will
  # never succeed on retry, so they should fail loudly. 429 is transient.
  static def check!(response, op, topic)
    status = response["status"]
    return if status == 200
    body = response["body"]
    case status
    when 401
      raise("es #{op} unauthorized — check ES_AUTH (topic=#{topic})")
    when 403
      raise("es #{op} forbidden — key lacks ACL for topic=#{topic}: #{body}")
    when 429
      raise("es #{op} rate-limited (topic=#{topic}): #{body}")
    else
      raise("es #{op} failed: #{status} #{body}")
    end
  end
end
```

A few choices worth flagging:

- **`value` is serialized at the boundary.** `es` records carry strings. We `.to_json` once in `emit` so callsites pass plain Soli hashes — `Es.emit("orders", order.id.to_s, {"id": order.id, "total": order.total})`.
- **`ES_AUTH` is optional, not required.** `headers` returns `{}` when the env var is unset, so the wrapper works against an unauthenticated dev broker without an `if dev …` branch.
- **No client-side retry.** A 5xx surfaces as a raised exception. The caller decides — a controller turns it into a 500 to the user; a job lets SolidB re-run it. 401/403 are surfaced distinctly because *no* retry policy will fix them; they mean "rotate the key or fix its ACL."
- **`commit` returns 204.** Treating any non-204 as failure catches bugs early. `es` is strict about this — silently accepting a 200 would mask a routing change.

## Step 3: Produce from a Controller

A signup flow that emits an `orders.created` event after the row is persisted. The user gets their redirect; the billing worker, the analytics pipeline, the welcome-email job — all the downstream consumers wake up on their own schedule.

```soli
# app/controllers/orders_controller.sl
class OrdersController
  def create
    order = Order.create(params["order"])
    return {"status": 422, "json": {"errors": order._errors}} if order._errors

    # Key by user_id so every order for the same user lands in the same
    # partition — that's how `es` (and Kafka) preserve per-user ordering.
    Es.emit("orders", order.user_id.to_s, {
      "event":      "order.created",
      "order_id":   order.id,
      "user_id":    order.user_id,
      "total":      order.total,
      "currency":   order.currency,
      "created_at": Time.now.iso8601
    })

    redirect("/orders/#{order.id}")
  end
end
```

Two design notes:

1. **The event is `order.created`, not `Charge stripe and email the user`.** Producers describe *what happened*; consumers decide *what to do*. That's what makes this scalable later — a new consumer (fraud scoring, say) joins the topic without the controller knowing it exists.
2. **The key is `user_id`, not `order.id`.** All of a single user's orders are guaranteed to be processed in submission order by a single consumer. Different users may interleave, which is exactly what you want for throughput.

If the broker is briefly down, the `HTTP.post_json` raises, the controller returns a 500, and the user retries. There's no in-memory queue eating the event. That's a deliberate tradeoff: at the cost of one extra network hop in the happy path, you get a story for failure that the user understands. If you need fire-and-forget producing, wrap the `Es.emit` in a `BillingEmitJob.perform_later({...})` — but most apps don't.

### Making Retries Safe with `producer_id`

The naive retry above has one subtle hole: what if the broker *did* accept the record and only the HTTP reply was lost? The user retries, the controller calls `Es.emit` again, and now there are two `order.created` events for the same order. The billing drain charges Stripe twice.

`es` closes that gap with opt-in idempotent producing. Tag a request with a stable `producer_id` and a monotonic `sequence` per `(producer_id, topic, partition)`, and the broker dedupes retries automatically:

```soli
class OrdersController
  # One producer_id per process. The broker remembers what sequences it has
  # already seen under this id, and a retry of the same (id, partition, seq)
  # returns the original offset with duplicate=true instead of re-appending.
  PRODUCER_ID = "web-#{getenv("HOSTNAME") || "local"}"

  def create
    order = Order.create(params["order"])
    return {"status": 422, "json": {"errors": order._errors}} if order._errors

    Es.emit("orders", order.user_id.to_s, {
      "event":    "order.created",
      "order_id": order.id,
      "user_id":  order.user_id,
      "total":    order.total
    }, PRODUCER_ID, order.id)

    redirect("/orders/#{order.id}")
  end
end
```

The mechanics worth knowing:

- **Sequence must be strictly increasing within a partition, with no gaps.** The broker rejects `expected N, got N+2` with `400 sequence gap`. Using `order.id` works as long as the database guarantees monotonic IDs per row; if two orders for different users land in different partitions (which is what user-keyed partitioning does), each partition just sees a *subset* of the global sequence, but each subset is still monotonic — which is what the broker checks.
- **Duplicates return the original offset, not a new one.** `ProduceResult.duplicate == true` tells you "I already had this; here's where it landed the first time." The drain side sees one record, not two.
- **Producer state is durable on the broker side.** It survives broker restarts (persisted alongside topic data). To wipe it — say, after rotating a `producer_id` — use `es producer revoke --id web-orders-1`.

If you don't need this, skip it. The argument is optional; controllers that call `Es.emit("orders", key, value)` without the extra args keep the old at-least-once behavior. Reach for `producer_id` when the *consumer* can't tolerate duplicates and the *producer* might retry — typically anything that drives an external side-effect (Stripe, SES, a webhook).

## Step 4: Drain with a Consumer Group

The consumer is the interesting half. We want:

- **Resumable** — restart the worker without re-processing what's already done.
- **Per-partition parallelism** — three partitions, three drains, three times the throughput.
- **At-least-once delivery** — handlers may run twice on a crash; they must be idempotent.

A consumer group on `es` gives us the first two for free; the third is on us.

```soli
# app/jobs/orders_drain_job.sl
class OrdersDrainJob
  GROUP_NAME = "billing"
  TOPIC      = "orders"
  BATCH_SIZE = 200

  static def perform(args)
    partition = args["partition"]

    loop do
      page = Es.consume(GROUP_NAME, TOPIC, partition, BATCH_SIZE)
      records = page["records"]

      break if records.blank?

      for record in records
        handle(record)
      end

      # Commit *after* every record in the batch is handled. If the worker
      # crashes mid-batch, the next run replays from the last commit — that's
      # the at-least-once guarantee.
      Es.commit(GROUP_NAME, TOPIC, partition, page["next_offset"])

      # If the page wasn't full, we've drained to the high-water mark.
      break if records.length < BATCH_SIZE
    end

    {"partition": partition, "drained_to": page["next_offset"]}
  end

  static def handle(record)
    payload = JSON.parse(record["value"])

    case payload["event"]
    when "order.created"
      BillingService.charge(payload["order_id"])
    when "order.refunded"
      BillingService.refund(payload["order_id"])
    end
  end
end
```

Three subtleties:

- **Commit happens after the handler returns, not before.** If `BillingService.charge` raises mid-batch, the loop unwinds without committing, and the next run sees the same records again. That's the at-least-once contract — combined with an idempotency key on the billing side (e.g. `order_id` as Stripe's `Idempotency-Key`), double delivery is harmless.
- **`break if records.blank?`** — `next_offset == high_watermark` shows up as an empty page. That's the natural signal to stop a drain.
- **`partition` is a job argument.** The scheduler runs one `OrdersDrainJob` per partition in parallel. Schedule them with a tiny dispatcher:

```soli
# app/jobs/orders_dispatcher_job.sl
class OrdersDispatcherJob
  static def perform(args)
    partition_count = Es.partition_count("orders")
    for p in 0..partition_count - 1
      OrdersDrainJob.perform_later({"partition": p})
    end
  end
end
```

Schedule the dispatcher every minute — or every five seconds, depending on how fresh you want the billing pipeline to be. The cost of a drain that finds nothing is one HTTP GET; cheap enough to poll aggressively if you want sub-second latency without leaning on long-poll.

### Letting the Broker Assign Partitions

The dispatcher above is fine when *you* know how many drain workers exist and which partitions each should handle. The moment you want to scale by adding worker processes — or survive a worker crashing without one partition's stream falling silent — the static fan-out stops being enough. You'd be reinventing the part of Kafka that does this for you.

`es` does it for you too, now. Every worker calls `join_group("billing", ["orders"])`, the broker assigns each one a slice of the partitions, and a heartbeat-based failure detector triggers an automatic rebalance when a worker dies. The drain code stays almost identical — only the *source* of `partition` changes from "a job argument I hard-coded" to "whatever the broker told me to drain on this generation."

```soli
# app/workers/orders_worker.sl
class OrdersWorker
  GROUP_NAME    = "billing"
  TOPIC         = "orders"
  HEARTBEAT_MS  = 5_000        # send before the coordinator's 30s timeout fires
  BATCH_SIZE    = 200

  static def run
    state = Es.join_group(GROUP_NAME, [TOPIC])
    member_id  = state["member_id"]
    generation = state["generation"]
    assignment = state["assignment"]

    log_info("joined #{GROUP_NAME} as #{member_id} (gen #{generation}): #{assignment}")

    last_beat = Time.now
    loop do
      for tp in assignment
        # tp is {"topic": "orders", "partition": N}
        drain_partition(tp["partition"])
      end

      # Heartbeat at most once per HEARTBEAT_MS — also our chance to learn that
      # the coordinator wants us to rebalance.
      if (Time.now - last_beat) * 1000 >= HEARTBEAT_MS
        beat = Es.heartbeat(GROUP_NAME, member_id, generation)
        last_beat = Time.now

        case beat["status"]
        when "ok"
          # carry on
        when "rebalance_required"
          log_info("rebalance triggered, re-joining")
          state      = Es.join_group(GROUP_NAME, [TOPIC], member_id)
          generation = state["generation"]
          assignment = state["assignment"]
        when "unknown_member"
          # We were evicted. Start fresh — coordinator will mint a new id.
          state      = Es.join_group(GROUP_NAME, [TOPIC])
          member_id  = state["member_id"]
          generation = state["generation"]
          assignment = state["assignment"]
        end
      end

      sleep(0.2)
    end
  ensure
    # Clean shutdown speeds up rebalance for the rest of the group from "30s
    # heartbeat timeout" to "immediate."
    Es.leave_group(GROUP_NAME, member_id) rescue nil
  end

  static def drain_partition(partition)
    loop do
      page = Es.consume(GROUP_NAME, TOPIC, partition, BATCH_SIZE)
      records = page["records"]
      break if records.blank?

      for record in records
        handle(record)
      end
      Es.commit(GROUP_NAME, TOPIC, partition, page["next_offset"])
      break if records.length < BATCH_SIZE
    end
  end

  static def handle(record)
    # … same as OrdersDrainJob.handle above
  end
end
```

What the coordinator gives you, beyond the static dispatcher:

- **Scaling by adding workers.** Run two `OrdersWorker` processes — each gets ~half the partitions. Run six — each gets one (for a 6-partition topic). No code changes, no redeploy.
- **Failover without manual intervention.** Stop sending heartbeats (worker crashed, network partitioned, GC pause too long) and within `member_timeout` (30s default) the coordinator evicts the dead member and reassigns its partitions to the survivors. The replays land at the survivor's next `Es.consume` — at-least-once guarantees do the rest.
- **A real shutdown story.** `Es.leave_group` in an `ensure` block turns a SIGTERM into an immediate rebalance, instead of the group waiting for the heartbeat timeout. Production rolling deploys depend on this.

When to keep the static dispatcher: single worker process, fixed deployment, you'd rather schedule via Soli's job runner than run a long-lived consumer. When to switch to `join_group`: anytime you want to scale horizontally or tolerate worker failure.

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

`es key revoke --id key_c91f30` flips the `disabled` flag in `./data/keys/api_keys.json` and the broker rejects further requests carrying that secret with `401`. The secret itself is never stored — only its SHA-256, so a stolen `api_keys.json` doesn't leak credentials, only their hashes.

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

The pieces sit on three deliberate boundaries:

- **`Es` doesn't know about orders or billing.** It speaks records. If you add a `payments` topic tomorrow, `Es.emit("payments", ...)` works without changes.
- **The controller doesn't know about billing.** It describes the event and is done. A new consumer (analytics, fraud) joins by subscribing — there's no editing the producer.
- **The drain job doesn't know about the broker.** It calls `Es.consume`. Swap `es` for Kafka tomorrow by reimplementing `Es` against Kafka's REST proxy and the job is untouched.
- **Auth lives entirely inside the wrapper.** Callers never reference `ES_AUTH` or `Authorization`. Rotating the writer's key is a deploy that changes one env var — no controller code moves. The wrapper is also where 401/403 get a *meaning*: "rotate the key" vs. "broker is sick" is a distinction the caller would otherwise have to learn.

That layering is also why the *whole thing* fits in three files plus a wrapper. Kafka's bigness is largely about features you don't have yet — cross-topic transactions, schema registries, geo-replication. `es` punts on all of them. If you ever need them, you migrate. Until then, you ship.

One subtlety worth knowing about even if you never touch it: `es-broker` can listen on a second port with `--bind-binary 127.0.0.1:9001`, exposing a length-framed binary protocol (`ES01` magic, opcodes for Produce/Consume/Ping) that carries native `Vec<u8>` keys and values. It exists for the case where JSON parse cost or UTF-8 round-tripping becomes a measurable bottleneck — typically a Rust-side ingestion service flushing millions of records a second. From Soli, stick with HTTP+JSON; you have HTTP builtins and not a custom-framing builtin, and the JSON path comfortably keeps up with anything a Soli app produces in a single request. The binary path is a future you can grow into without rewriting the producer/consumer model.

## When to Reach for `es` vs. What You Already Have

Soli has two adjacent tools that overlap with `es`:

- **SolidB-backed background jobs** (covered in [Sending Email with SendGrid](/docs/blog/sendgrid-email-jobs)) — durable, retried, scheduled. The difference: a job is a *unit of work to do once*; a topic is a *stream of facts other systems will read repeatedly, in order*. Use jobs for "send this email"; use `es` for "every order ever placed, replayable by any consumer."
- **WebSocket broadcasts** — pushed to connected clients, ephemeral, no replay. Use WS for "tell every open dashboard a new order came in"; use `es` for "let the billing service catch up on the last hour of orders after a deploy."

You'll often want both. A controller can `Es.emit` and `ws_broadcast` in the same request: the event lands in the durable log for the billing drain, *and* the live order board updates in real time. The two are orthogonal.

## What's Next

A few directions to extend this:

- **Dead-letter handling.** A handler that has raised three times in a row probably won't succeed on the fourth. Track per-record attempt counts in a SolidB collection keyed by `(topic, partition, offset)` and route the record to a `dead_letters` topic instead of blocking the drain.
- **Schema validation.** A `to_h` round-trip is generous about what comes in. Validate the payload's shape with `User.validate(...)` (or a JSON Schema check) inside `handle` and route bad records to a `parse_failures` topic.
- **Compacted topics for current-state streams.** When the stream represents *state* rather than *facts* — `user.profile_updated`, `cart.contents_changed`, `account.balance` — you don't want the full history, you want the latest value per key. Create the topic with `--cleanup-policy compact`, write the current state as the `value` (with `null` value to tombstone a key), and the broker keeps only the most recent record per key forever:

  ```bash
  $ es topic create --name user_profiles --partitions 3 --cleanup-policy compact
  ```

  A late-joining consumer reads from offset 0 and gets a snapshot of every user that has ever existed, in O(distinct keys) records instead of O(history). Combine with `--cleanup-policy compact,delete` if you also want a time bound — e.g. "latest per user, but drop users untouched for 90 days."

The producer is one line. The consumer is fifteen. The broker is one process. That ratio — three small, well-named pieces — is what makes event streaming feel like a tool rather than an architecture decision.
