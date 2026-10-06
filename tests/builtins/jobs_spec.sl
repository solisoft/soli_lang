# Background jobs: the low-level Job queue API. The queue lives in the `_jobs`
# collection, which persists between tests, so every enqueued id is cancelled
# after each test. The per-class facade (perform_later…) is only injected into
# app/jobs/*_job.sl classes. Cron: cron_spec.sl; Webhook: webhook_spec.sl.

enqueued_ids = []

def track(id)
  enqueued_ids.push(id)
  id
end

def jobs_for(handler)
  Job.list().filter { |job| job["handler"] == handler }
end

describe("Job argument validation") do
  test("enqueue needs a handler and args") do
    assert_raises("Job.enqueue(handler, args, queue_or_opts?) requires at least 2 arguments") do
      Job.enqueue("OnlyHandler")
    end
  end

  test("enqueue refuses a handler that is not a string") do
    assert_raises("Job.enqueue() expects string at position 1, got int") do
      Job.enqueue(42, {})
    end
  end

  test("enqueue_in needs a handler, a duration and args") do
    assert_raises("Job.enqueue_in(handler, duration, args, queue_or_opts?) requires at least 3 arguments") do
      Job.enqueue_in("SomeJob", "5 minutes")
    end
  end

  test("enqueue_in refuses an unknown duration unit") do
    assert_raises("unknown duration unit: fortnights") do
      Job.enqueue_in("SomeJob", "5 fortnights", {})
    end
  end

  test("enqueue_at refuses a string that is not an ISO-8601 timestamp") do
    assert_raises("invalid datetime \"not-a-timestamp\": expected an ISO-8601 timestamp") do
      Job.enqueue_at("SomeJob", "not-a-timestamp", {})
    end
  end
end

describe("Job queue") do
  before_each() do
    enqueued_ids = []
    requires_solidb()
  end

  after_each() do
    enqueued_ids.each do |id|
      Job.cancel(id)
    end
  end

  describe("enqueue") do
    test("returns the id of a pending row in the default queue") do
      id = track(Job.enqueue("SpecEnqueueJob", {"n": 1}))
      jobs = jobs_for("SpecEnqueueJob")
      assert_eq(jobs.length, 1)
      job = jobs[0]
      assert_eq(job["_key"], id)
      assert_eq(job["args"], {"n": 1})
      assert_eq(job["queue"], "default")
      assert_eq(job["state"], "pending")
      assert_eq(job["priority"], 0)
      assert_eq(job["attempts"], 0)
      assert_eq(job["max_retries"], 3)
    end

    test("takes a queue name as the trailing argument") do
      track(Job.enqueue("SpecQueueNameJob", {}, "spec_named_queue"))
      assert_eq(jobs_for("SpecQueueNameJob")[0]["queue"], "spec_named_queue")
    end

    test("takes an options hash with queue, priority and max_retries") do
      track(Job.enqueue("SpecOptionsJob", {"n": 2}, {"queue": "spec_queue", "priority": 5, "max_retries": 7}))
      job = jobs_for("SpecOptionsJob")[0]
      assert_eq(job["queue"], "spec_queue")
      assert_eq(job["priority"], 5)
      assert_eq(job["max_retries"], 7)
    end
  end

  describe("scheduling") do
    test("enqueue_in stores a scheduled row in the future") do
      track(Job.enqueue_in("SpecDelayedJob", "10 minutes", {}))
      job = jobs_for("SpecDelayedJob")[0]
      assert_eq(job["state"], "scheduled")
      delay = DateTime.parse(job["run_at"]).to_unix - DateTime.parse(job["created_at"]).to_unix
      assert_eq(delay, 600)
    end

    test("enqueue_at stores the given run_at") do
      track(Job.enqueue_at("SpecScheduledJob", "2038-01-01T00:00:00Z", {}))
      job = jobs_for("SpecScheduledJob")[0]
      assert_eq(job["state"], "scheduled")
      assert_eq(job["run_at"], "2038-01-01T00:00:00Z")
    end
  end

  describe("list and queues") do
    test("list filters by queue") do
      track(Job.enqueue("SpecListJob", {}, "spec_list_queue"))
      track(Job.enqueue("SpecOtherQueueJob", {}, "spec_other_queue"))
      handlers = Job.list("spec_list_queue").map { |job| job["handler"] }
      assert_eq(handlers, ["SpecListJob"])
    end

    test("list is empty for a queue with no work") do
      assert_eq(Job.list("spec_empty_queue"), [])
    end

    test("queues names the queues holding work") do
      track(Job.enqueue("SpecQueuesJob", {}, "spec_queues_queue"))
      assert_contains(Job.queues, "spec_queues_queue")
    end
  end

  describe("cancel and retry") do
    test("cancel removes a pending job, then reports false") do
      id = Job.enqueue("SpecCancelJob", {})
      assert_eq(Job.cancel(id), true)
      assert_eq(jobs_for("SpecCancelJob"), [])
      assert_eq(Job.cancel(id), false)
    end

    test("cancel returns false for an unknown id") do
      assert_eq(Job.cancel("spec-no-such-job"), false)
    end

    test("retry returns false for an unknown id") do
      assert_eq(Job.retry("spec-no-such-job"), false)
    end

    test("retry refuses a pending job") do
      id = track(Job.enqueue("SpecPendingJob", {}))
      assert_raises("is pending and cannot be retried") do
        Job.retry(id)
      end
    end
  end
end
