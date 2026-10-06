# Semaphore.reset, the sticky limit and argument checks (in memory), plus the
# Cron.update / Cron.delete and Webhook.cancel corners against SoliDB that
# new_builtins_spec.sl, cron_spec.sl and webhook_spec.sl leave out.

SEMAPHORE_NAMES = ["extras-sem-a", "extras-sem-b", "extras-sem-c"]
CRON_NAMES = ["extras_cron_a", "extras_cron_b"]
HOOK_URL = "https://example.invalid/hook"

def extras_crons
  Cron.list.select do |entry|
    CRON_NAMES.includes?(entry.name)
  end
end

def extras_cron(name)
  extras_crons.find do |entry|
    entry.name == name
  end
end

describe("Semaphore") do
  after_each() do
    SEMAPHORE_NAMES.each do |name|
      Semaphore.reset(name)
    end
  end

  context("reset") do
    test("drops the name and every token held on it") do
      Semaphore.try_acquire("extras-sem-a", 1)
      assert_null(Semaphore.try_acquire("extras-sem-a", 1))
      assert(Semaphore.reset("extras-sem-a"))
      assert_null(Semaphore.count("extras-sem-a"))
    end

    test("frees the slot of a leaked token") do
      leaked = Semaphore.try_acquire("extras-sem-a", 1)
      Semaphore.reset("extras-sem-a")
      fresh = Semaphore.try_acquire("extras-sem-a", 1)
      assert_ne(fresh, leaked)
      counts = Semaphore.count("extras-sem-a")
      assert_eq(
        counts,
        {"limit": 1, "held": 1}
      )
    end

    test("a released token of the dropped name no longer counts") do
      leaked = Semaphore.try_acquire("extras-sem-a", 1)
      Semaphore.reset("extras-sem-a")
      assert_not(Semaphore.release("extras-sem-a", leaked))
    end

    test("lets the next caller fix a new limit") do
      Semaphore.try_acquire("extras-sem-b", 1)
      Semaphore.reset("extras-sem-b")
      Semaphore.try_acquire("extras-sem-b", 3)
      counts = Semaphore.count("extras-sem-b")
      assert_eq(
        counts,
        {"limit": 3, "held": 1}
      )
    end

    test("answers false for a name that does not exist") do
      assert_not(Semaphore.reset("extras-sem-never-used"))
    end

    test("answers false the second time") do
      Semaphore.try_acquire("extras-sem-c", 1)
      assert(Semaphore.reset("extras-sem-c"))
      assert_not(Semaphore.reset("extras-sem-c"))
    end
  end

  context("limit") do
    test("the first caller's limit sticks") do
      Semaphore.try_acquire("extras-sem-a", 1)
      assert_null(Semaphore.try_acquire("extras-sem-a", 10))
      counts = Semaphore.count("extras-sem-a")
      assert_eq(
        counts,
        {"limit": 1, "held": 1}
      )
    end

    test("survives every token being released") do
      token = Semaphore.try_acquire("extras-sem-a", 1)
      Semaphore.release("extras-sem-a", token)
      Semaphore.try_acquire("extras-sem-a", 5)
      counts = Semaphore.count("extras-sem-a")
      assert_eq(
        counts,
        {"limit": 1, "held": 1}
      )
    end
  end

  context("argument checks") do
    test("try_acquire refuses a zero or non-int limit") do
      assert_raises("Semaphore.try_acquire() expects a positive Int limit, got int") do
        Semaphore.try_acquire("extras-sem-a", 0)
      end
      assert_raises("Semaphore.try_acquire() expects a positive Int limit, got string") do
        Semaphore.try_acquire("extras-sem-a", "2")
      end
      assert_null(Semaphore.count("extras-sem-a"))
    end

    test("try_acquire and reset refuse a non-string name") do
      assert_raises("Semaphore.try_acquire() name expects a string, got int") do
        Semaphore.try_acquire(5, 2)
      end
      assert_raises("Semaphore.reset() name expects a string, got int") do
        Semaphore.reset(5)
      end
    end

    test("release refuses a non-int token and answers false for an unknown name") do
      assert_raises("Semaphore.release() expects an Int token, got string") do
        Semaphore.release("extras-sem-a", "token")
      end
      assert_not(Semaphore.release("extras-sem-never-used", 1))
    end
  end
end

describe("Cron entries") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    extras_crons.each do |entry|
      Cron.delete(entry.name)
    end
  end

  context("update") do
    test("disabling an entry answers true and is stored") do
      Cron.schedule("extras_cron_a", "0 0 3 * * *", "ReportJob")
      assert(Cron.update("extras_cron_a", {"enabled": false}))
      assert_eq(extras_cron("extras_cron_a").enabled, false)
    end

    test("raises for a name that was never scheduled") do
      assert_raises("Cron.update failed: HTTP 404") do
        Cron.update("extras_cron_b", {"enabled": false})
      end
      assert_null(extras_cron("extras_cron_b"))
    end
  end

  context("delete") do
    test("leaves the other entries in place") do
      Cron.schedule("extras_cron_a", "0 0 3 * * *", "ReportJob")
      Cron.schedule("extras_cron_b", "0 0 4 * * *", "ReportJob")
      Cron.delete("extras_cron_a")
      remaining = extras_crons.map do |entry|
        entry.name
      end
      assert_eq(remaining, ["extras_cron_b"])
    end

    test("refuses a non-string name") do
      assert_raises("Cron.delete() expects string at position 1, got int") do
        Cron.delete(5)
      end
    end
  end
end

describe("Webhook.cancel") do
  before_each() do
    requires_solidb()
  end

  test("a cancelled delivery leaves its queue") do
    id = Webhook.enqueue(HOOK_URL, {"event": "created"}, {"queue": "extras_hooks"})
    assert_eq(Webhook.list("extras_hooks").length, 1)
    assert(Webhook.cancel(id))
    assert_eq(Webhook.list("extras_hooks"), [])
  end

  test("answers false for an id that was never enqueued") do
    assert_not(Webhook.cancel("extras-no-such-job"))
  end

  test("cancels a plain job too: both share the _jobs collection") do
    id = Job.enqueue("ExtrasJob", {}, "extras_jobs")
    assert(Webhook.cancel(id))
    assert_eq(Job.list("extras_jobs"), [])
  end

  test("refuses a non-string id") do
    assert_raises("expects string at position 1, got int") do
      Webhook.cancel(5)
    end
  end
end
