# Webhook: outgoing HTTP deliveries queued as `__WebhookDelivery` rows in the
# same `_jobs` collection as Job. Rows persist between tests, so every id is
# cancelled after each one.

const HOOK_URL = "https://example.com/hook"

enqueued_ids = []

def track(id)
  enqueued_ids.push(id)
  id
end

def delivery(id)
  Webhook.list().find { |job| job["_key"] == id }
end

describe("Webhook argument validation") do
  test("enqueue needs a url and a payload") do
    assert_raises("Webhook.enqueue(url, payload, opts?) requires at least 2 arguments") do
      Webhook.enqueue(HOOK_URL)
    end
  end

  test("enqueue_in needs a url, a duration and a payload") do
    assert_raises("Webhook.enqueue_in(url, duration, payload, opts?) requires at least 3 arguments") do
      Webhook.enqueue_in(HOOK_URL, "5 minutes")
    end
  end

  test("enqueue_at refuses a string that is not an ISO-8601 timestamp") do
    assert_raises("invalid datetime \"garbage\": expected an ISO-8601 timestamp") do
      Webhook.enqueue_at(HOOK_URL, "garbage", {})
    end
  end
end

describe("Webhook deliveries") do
  before_each() do
    enqueued_ids = []
    requires_solidb()
  end

  after_each() do
    enqueued_ids.each do |id|
      Webhook.cancel(id)
    end
  end

  test("enqueue stores a pending delivery with the url and secret") do
    id = track(Webhook.enqueue(HOOK_URL, {"event": "ping"}, {"secret": "shh"}))
    row = delivery(id)
    assert_eq(row["handler"], "__WebhookDelivery")
    assert_eq(row["args"], {"event": "ping"})
    assert_eq(row["webhook"], {"url": HOOK_URL, "secret": "shh"})
    assert_eq(row["state"], "pending")
  end

  test("enqueue_in schedules the delivery for later") do
    id = track(Webhook.enqueue_in(HOOK_URL, "1 minute", {"event": "later"}))
    row = delivery(id)
    assert_eq(row["state"], "scheduled")
    delay = DateTime.parse(row["run_at"]).to_unix - DateTime.parse(row["created_at"]).to_unix
    assert_eq(delay, 60)
  end

  test("enqueue_at schedules the delivery at a timestamp") do
    id = track(Webhook.enqueue_at(HOOK_URL, "2038-01-01T00:00:00Z", {"event": "then"}))
    row = delivery(id)
    assert_eq(row["state"], "scheduled")
    assert_eq(row["run_at"], "2038-01-01T00:00:00Z")
  end

  test("list includes every queued delivery") do
    first = track(Webhook.enqueue(HOOK_URL, {"n": 1}))
    second = track(Webhook.enqueue(HOOK_URL, {"n": 2}))
    keys = Webhook.list().map { |job| job["_key"] }
    assert_contains(keys, first)
    assert_contains(keys, second)
  end

  test("cancel removes a delivery") do
    id = Webhook.enqueue(HOOK_URL, {"event": "cancelled"})
    assert_eq(Webhook.cancel(id), true)
    assert_null(delivery(id))
    assert_eq(Webhook.cancel(id), false)
  end
end
