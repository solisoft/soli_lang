// ============================================================================
// Jobs / Webhooks / RateLimiter / SSE / WebSocket / Native / Updater / Push /
// FCM / VAPID / LLM / Auth & Response helpers — extended coverage spec
// ============================================================================
//
// Offline-first coverage for APIs not exercised elsewhere:
//
// - `Job` / `Webhook` queue paths persist to SolidB (default localhost:6745),
//   so every DB-backed test is skip-guarded with an availability probe
//   (kv_spec style). Validation-only paths are tested unconditionally.
// - `RateLimiter`, SSE, WS argument validation, `Native`, `Updater`,
//   `Push.deliver` (no-target path), VAPID/FCM validation, router internals,
//   auth test-helpers and response builders are all in-memory or pure and run
//   everywhere.
//
// Existing coverage NOT duplicated here:
// - Cron expression builders + class registration (jobs_spec.sl)
// - RateLimiter instance creation / status keys / headers keys (rate_limit_spec.sl)
// ============================================================================

// ---------------------------------------------------------------------------
// Availability probes (run once, at load)
// ---------------------------------------------------------------------------

let __jobs_db_available = false
try {
    Job.enqueue("__spec__probe_job", {});
    __jobs_db_available = true;
} catch e {
    // No SolidB answering on localhost:6745 — queue paths stay skipped.
}

// A channel token can only be minted when a session secret is configured.
let __native_token = null
try {
    __native_token = Native.channel_token("__spec__probe");
} catch e {
    __native_token = null;
}

// Embeddings need SOLI_EMBEDDING_API_KEY; without it the builtins fail fast
// with a config error before touching the network.
let __embedding_configured = hasenv("SOLI_EMBEDDING_API_KEY")

fn skip_unless_jobs_db() {
    if not __jobs_db_available {
        return null;
    }
}

describe("Job enqueue validation (offline)", fn() {
    test("Job.enqueue requires handler and args", fn() {
        let raised = false;
        try {
            Job.enqueue("OnlyHandler");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("Job.enqueue rejects non-string handler", fn() {
        let raised = false;
        try {
            Job.enqueue(42, {});
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("Job.enqueue_in requires three arguments", fn() {
        let raised = false;
        try {
            Job.enqueue_in("SomeJob", "5 minutes");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("Job.enqueue_in rejects unknown duration units", fn() {
        let raised = false;
        try {
            Job.enqueue_in("SomeJob", "5 fortnights", {});
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("Job.enqueue_at rejects invalid datetime strings", fn() {
        let raised = false;
        try {
            Job.enqueue_at("SomeJob", "not-a-timestamp", {});
        } catch e {
            raised = true;
        }
        assert(raised);
    });
});

describe("Job queue paths (SolidB-backed)", fn() {
    test("Job.enqueue returns a string job id", fn() {
        if not __jobs_db_available { return null; }
        let id = Job.enqueue("__spec__echo_job", {"n": 1});
        assert(type(id) == "string");
        assert(id.length() > 0);
    });

    test("Job.enqueue accepts an options hash with priority", fn() {
        if not __jobs_db_available { return null; }
        let id = Job.enqueue("__spec__echo_job", {"n": 2}, {"queue": "spec_queue", "priority": 5});
        assert(type(id) == "string");
    });

    test("Job.list finds the enqueued job by queue", fn() {
        if not __jobs_db_available { return null; }
        Job.enqueue("__spec__list_probe_job", {}, "spec_queue");
        let jobs = Job.list("spec_queue");
        assert(type(jobs) == "array");
        let found = false;
        for job in jobs {
            if job["handler"] == "__spec__list_probe_job" {
                found = true;
            }
        }
        assert(found);
    });

    test("Job.queues reports a non-empty array after enqueue", fn() {
        if not __jobs_db_available { return null; }
        Job.enqueue("__spec__queues_probe_job", {}, "spec_queue");
        let queues = Job.queues();
        assert(type(queues) == "array");
        assert(queues.length() > 0);
    });

    test("Job.enqueue_in schedules with a duration string", fn() {
        if not __jobs_db_available { return null; }
        let id = Job.enqueue_in("__spec__delayed_job", "10 minutes", {});
        assert(type(id) == "string");
    });

    test("Job.enqueue_at schedules with an ISO timestamp", fn() {
        if not __jobs_db_available { return null; }
        let id = Job.enqueue_at("__spec__scheduled_job", "2038-01-01T00:00:00Z", {});
        assert(type(id) == "string");
    });

    test("Job.retry returns false for an unknown id", fn() {
        if not __jobs_db_available { return null; }
        assert_eq(Job.retry("__spec__no_such_job_id"), false);
    });

    test("Job.retry refuses a pending job", fn() {
        if not __jobs_db_available { return null; }
        let id = Job.enqueue("__spec__pending_job", {});
        let raised = false;
        try {
            Job.retry(id);
        } catch e {
            raised = str(e).includes?("cannot be retried");
        }
        assert(raised);
    });

    test("Job.cancel cancels a pending job and reports false twice", fn() {
        if not __jobs_db_available { return null; }
        let id = Job.enqueue("__spec__cancel_me_job", {});
        assert_eq(Job.cancel(id), true);
        // Already cancelled → unknown id on the second call.
        assert_eq(Job.cancel(id), false);
    });
});

describe("Webhook class", fn() {
    test("Webhook.enqueue requires url and payload", fn() {
        let raised = false;
        try {
            Webhook.enqueue("https://example.com/hook");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("Webhook.enqueue_in requires three arguments", fn() {
        let raised = false;
        try {
            Webhook.enqueue_in("https://example.com/hook", "5 minutes");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("Webhook.enqueue_at rejects invalid datetime strings", fn() {
        let raised = false;
        try {
            Webhook.enqueue_at("https://example.com/hook", "garbage", {});
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("Webhook.enqueue returns a string job id", fn() {
        if not __jobs_db_available { return null; }
        let id = Webhook.enqueue("https://example.com/hook", {"event": "ping"}, {"secret": "shh"});
        assert(type(id) == "string");
    });

    test("Webhook.enqueue_in returns a string job id", fn() {
        if not __jobs_db_available { return null; }
        let id = Webhook.enqueue_in("https://example.com/hook", "1 minute", {"event": "later"});
        assert(type(id) == "string");
    });

    test("Webhook.enqueue_at returns a string job id", fn() {
        if not __jobs_db_available { return null; }
        let id = Webhook.enqueue_at("https://example.com/hook", "2038-01-01T00:00:00Z", {"event": "then"});
        assert(type(id) == "string");
    });

    test("Webhook.list returns an array", fn() {
        if not __jobs_db_available { return null; }
        let hooks = Webhook.list();
        assert(type(hooks) == "array");
    });
});

describe("RateLimiter behavior (in-memory)", fn() {
    test("allowed goes false once the limit is exhausted", fn() {
        RateLimiter.reset_all();
        let req = {"headers": {"x-forwarded-for": "10.55.0.1"}};
        let rl = rate_limiter_from_ip(req, 2, 60);
        assert_eq(rl.allowed(), true);
        assert_eq(rl.allowed(), true);
        assert_eq(rl.allowed(), false);
        RateLimiter.reset_all();
    });

    test("buckets are per-key: another ip is unaffected", fn() {
        RateLimiter.reset_all();
        let first = rate_limiter_from_ip({"remote_addr": "10.55.0.2"}, 1, 60);
        assert_eq(first.allowed(), true);
        assert_eq(first.allowed(), false);
        let second = rate_limiter_from_ip({"remote_addr": "10.55.0.3"}, 1, 60);
        assert_eq(second.allowed(), true);
        RateLimiter.reset_all();
    });

    test("status reports allowed=false and remaining=0 when exhausted", fn() {
        RateLimiter.reset_all();
        let rl = rate_limiter_from_ip({"remote_addr": "10.55.0.4"}, 1, 60);
        let _ = rl.allowed();
        let s = rl.status();
        assert_eq(s["allowed"], false);
        assert_eq(s["remaining"], 0);
        assert_eq(s["limit"], 1);
        assert_eq(s["window"], 60);
        RateLimiter.reset_all();
    });

    // NOTE: the instance-level `reset()` is shadowed by the `reset` field
    // `rate_limiter_from_ip` stamps on every instance (instance fields win
    // over native methods), so bucket resets are exercised through the static
    // `reset_all()` here.
    test("reset_all() reopens an exhausted bucket for the same key", fn() {
        RateLimiter.reset_all();
        let rl = rate_limiter_from_ip({"remote_addr": "10.55.0.5"}, 1, 60);
        assert_eq(rl.allowed(), true);
        assert_eq(rl.allowed(), false);
        RateLimiter.reset_all();
        let fresh = rate_limiter_from_ip({"remote_addr": "10.55.0.5"}, 1, 60);
        assert_eq(fresh.allowed(), true);
    });

    test("reset_all() clears every bucket", fn() {
        let rl = rate_limiter_from_ip({"remote_addr": "10.55.0.6"}, 1, 60);
        let _ = rl.allowed();
        RateLimiter.reset_all();
        let fresh = rate_limiter_from_ip({"remote_addr": "10.55.0.6"}, 1, 60);
        assert_eq(fresh.allowed(), true);
    });

    test("cleanup() returns true", fn() {
        assert_eq(RateLimiter.cleanup(), true);
    });

    test("a zero limit always allows", fn() {
        RateLimiter.reset_all();
        let rl = rate_limiter_from_ip({"remote_addr": "10.55.0.7"}, 0, 60);
        assert_eq(rl.allowed(), true);
        assert_eq(rl.allowed(), true);
        RateLimiter.reset_all();
    });

    test("throttle() reports wait seconds while throttled", fn() {
        RateLimiter.reset_all();
        let rl = rate_limiter_from_ip({"remote_addr": "10.55.0.8"}, 60, 60);
        let wait = rl.throttle();
        assert(type(wait) == "int");
        RateLimiter.reset_all();
    });
});

describe("Deprecated rate-limit globals", fn() {
    test("each removed global explains the migration", fn() {
        for msg in [rate_limit(), throttle(), rate_limit_ip(), rate_limit_status(),
                    rate_limit_reset(), rate_limit_reset_all(), rate_limit_cleanup(),
                    rate_limit_headers()] {
            assert(type(msg) == "string");
            assert(str(msg).includes?("removed"));
            assert(str(msg).includes?("RateLimiter"));
        }
    });
});

describe("SSE pub/sub (in-memory)", fn() {
    test("broadcast to a topic with no subscribers reaches 0 clients", fn() {
        assert_eq(sse_broadcast("spec_quiet_topic", "hello"), 0);
    });

    test("subscriber count is 0 with nobody listening", fn() {
        assert_eq(sse_subscribers("spec_quiet_topic"), 0);
    });

    test("sse_subscribers still reports 0 after a broadcast", fn() {
        assert_eq(sse_broadcast("spec_other_topic", "data", "custom-event"), 0);
        assert_eq(sse_subscribers("spec_other_topic"), 0);
    });

    test("sse_subscribers rejects a missing topic", fn() {
        let raised = false;
        try {
            sse_subscribers(42);
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("sse_broadcast rejects a missing topic", fn() {
        let raised = false;
        try {
            sse_broadcast(null, "data");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("sse without a block raises a clean error", fn() {
        let raised = false;
        try {
            sse({});
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("sse_subscribe without a topic raises a clean error", fn() {
        let raised = false;
        try {
            sse_subscribe({});
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("stream without a block raises a clean error", fn() {
        let raised = false;
        try {
            stream({}, "text/csv");
        } catch e {
            raised = true;
        }
        assert(raised);
    });
});

describe("WebSocket helpers (validation + offline counts)", fn() {
    test("ws_count is 0 outside a server", fn() {
        assert_eq(ws_count(), 0);
    });

    test("ws_clients returns a hash", fn() {
        assert(type(ws_clients()) == "hash");
    });

    test("ws_send rejects a non-string connection id", fn() {
        let raised = false;
        try {
            ws_send(42, "hello");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("ws_close rejects a malformed connection id", fn() {
        let raised = false;
        try {
            ws_close("definitely-not-a-uuid", "reason");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("ws_broadcast_room rejects a non-string channel", fn() {
        let raised = false;
        try {
            ws_broadcast_room(42, "message");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("ws_list_presence rejects a non-string channel", fn() {
        let raised = false;
        try {
            ws_list_presence(null);
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("ws_presence_count rejects a non-string channel", fn() {
        let raised = false;
        try {
            ws_presence_count([]);
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("ws_get_presence rejects non-string arguments", fn() {
        let raised = false;
        try {
            ws_get_presence("room", 42);
        } catch e {
            raised = true;
        }
        assert(raised);
    });
});

describe("Native desktop channel", fn() {
    test("notify to an empty channel reaches 0 clients", fn() {
        assert_eq(Native.notify("spec:nobody", {"title": "Hi"}), 0);
    });

    test("subscribers is 0 with no live page open", fn() {
        assert_eq(Native.subscribers("spec:nobody"), 0);
    });

    test("notify rejects channels containing '.'", fn() {
        let raised = false;
        try {
            Native.notify("bad.channel", {"title": "Hi"});
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("subscribers rejects empty channels", fn() {
        let raised = false;
        try {
            Native.subscribers("");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("channel_token mints a three-part signed token when configured", fn() {
        if __native_token == null { return null; }
        assert(type(__native_token) == "string");
        assert(__native_token.split(".").length() == 3);
    });

    test("channel_token differs between channels when configured", fn() {
        if __native_token == null { return null; }
        let other = Native.channel_token("spec:somebody-else");
        assert(other != __native_token);
    });
});

describe("Updater (no auto-update channel in dev)", fn() {
    test("version is null outside a built artifact", fn() {
        assert_null(Updater.version());
    });

    test("check reports unconfigured, not available", fn() {
        let info = Updater.check();
        assert_eq(info["available"], false);
        assert_eq(info["configured"], false);
        assert(str(info["error"]).includes?("--update-url"));
    });

    test("apply reports not-configured status", fn() {
        let result = Updater.apply();
        assert_eq(result["status"], "not-configured");
    });
});

describe("Push.deliver (no targets configured)", fn() {
    test("deliver with no targets and no live bridge reaches nobody", fn() {
        let result = Push.deliver("spec:push-channel", {"title": "Hello"});
        assert_eq(result["reached_live"], 0);
        assert_eq(result["transport"], "none");
        assert(type(result["sent"]) == "array");
        assert(type(result["failed"]) == "array");
        assert(type(result["prune"]) == "array");
    });

    test("deliver validates its arguments", fn() {
        let raised = false;
        try {
            Push.deliver("only-channel");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("deliver rejects non-hash options", fn() {
        let raised = false;
        try {
            Push.deliver("spec:push-channel", {"title": "x"}, "bogus");
        } catch e {
            raised = true;
        }
        assert(raised);
    });
});

describe("FCM offline validation", fn() {
    test("send rejects implausible device tokens", fn() {
        let raised = false;
        try {
            Fcm.send("", {}, {});
        } catch e {
            raised = str(e).includes?("implausible device token");
        }
        assert(raised);
    });

    test("access_token rejects invalid service-account JSON", fn() {
        let raised = false;
        try {
            Fcm.access_token("{ definitely not json");
        } catch e {
            raised = str(e).includes?("service-account JSON");
        }
        assert(raised);
    });
});

describe("VAPID offline validation", fn() {
    test("vapid_generate_keys returns a base64url keypair", fn() {
        let keys = vapid_generate_keys();
        assert(type(keys["public_key"]) == "string");
        assert(type(keys["private_key"]) == "string");
        assert(keys["public_key"].length() > 40);
    });

    test("vapid_send arity errors are clean", fn() {
        let keys = vapid_generate_keys();
        let raised = false;
        try {
            vapid_send({"endpoint": "https://127.0.0.1:9/x"}, "payload",
                       keys["private_key"], keys["public_key"]);
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("vapid_send rejects subscriptions without keys", fn() {
        let keys = vapid_generate_keys();
        let raised = false;
        try {
            vapid_send({"endpoint": "https://127.0.0.1:9/x"}, "payload",
                       keys["private_key"], keys["public_key"], "mailto:test@example.com");
        } catch e {
            raised = true;
        }
        assert(raised);
    });
});

describe("LLM and embedding primitives (offline paths)", fn() {
    test("llm_generate rejects non-string prompts", fn() {
        let raised = false;
        try {
            llm_generate(42, "user prompt");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("embed rejects non-string input", fn() {
        let raised = false;
        try {
            embed(42);
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("embed_batch rejects arrays with non-strings", fn() {
        let raised = false;
        try {
            embed_batch(["ok", 42]);
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("embed fails fast with a config error when unconfigured", fn() {
        if __embedding_configured { return null; }
        let message = "";
        let raised = false;
        try {
            embed("some text");
        } catch e {
            raised = true;
            message = str(e);
        }
        assert(raised);
        assert(message.includes?("SOLI_EMBEDDING_API_KEY"));
    });

    test("embed_batch fails fast with a config error when unconfigured", fn() {
        if __embedding_configured { return null; }
        let message = "";
        let raised = false;
        try {
            embed_batch(["a", "b"]);
        } catch e {
            raised = true;
            message = str(e);
        }
        assert(raised);
        assert(message.includes?("SOLI_EMBEDDING_API_KEY"));
    });
});

describe("Auth test helpers (offline state machine)", fn() {
    test("signed_in is false by default", fn() {
        as_guest();
        assert_eq(signed_in?(), false);
        assert_eq(signed_in(), false);
        assert_eq(signed_out(), true);
        assert_eq(signed_out?(), true);
    });

    test("as_user sets the current user thread-local", fn() {
        as_guest();
        as_user(9);
        assert_eq(signed_in(), true);
        assert_eq(current_user()["id"], 9);
        as_guest();
    });

    test("as_admin signs in user 1", fn() {
        as_guest();
        as_admin();
        assert_eq(current_user()["id"], 1);
        as_guest();
    });

    test("as_guest clears authentication", fn() {
        as_user(5);
        as_guest();
        assert_null(current_user());
        assert_eq(signed_out(), true);
    });

    test("create_session marks the session and destroy_session clears it", fn() {
        as_guest();
        let sid = create_session(77);
        assert_eq(sid, "session_test_77");
        assert_eq(signed_in(), true);
        destroy_session();
        assert_eq(signed_in(), false);
    });

    test("login without a running test server raises a clean error", fn() {
        let raised = false;
        try {
            login("user@example.com", "secret");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("with_token and clear_authorization accept tokens without error", fn() {
        assert_null(with_token("abc123"));
        clear_authorization();
    });
});

describe("Response builders (offline)", fn() {
    test("render_text returns null and needs text", fn() {
        assert_null(render_text("plain body"));
        let raised = false;
        try {
            render_text();
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("render_jsonp falls back to JSON without a request callback", fn() {
        assert_null(render_jsonp({"answer": 42}));
    });

    test("redirect builds a 302 to a local path", fn() {
        let r = redirect("/dashboard");
        assert_eq(r["status"], 302);
        assert_eq(r["headers"]["Location"], "/dashboard");
    });

    test("redirect refuses off-site URLs", fn() {
        let raised = false;
        try {
            redirect("https://evil.example.com/phish");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("redirect_external allows trusted external URLs", fn() {
        let r = redirect_external("https://payments.example.net/checkout");
        assert_eq(r["status"], 302);
        assert_eq(r["headers"]["Location"], "https://payments.example.net/checkout");
    });

    test("halt builds a status/body response hash", fn() {
        let r = halt(404, "Not here");
        assert_eq(r["status"], 404);
        assert_eq(r["body"], "Not here");
    });

    test("forbidden raises instead of returning", fn() {
        let raised = false;
        try {
            forbidden("no access");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("csrf_token returns a stable non-empty token", fn() {
        let first = csrf_token();
        assert(type(first) == "string");
        assert(first.length() > 0);
        assert_eq(csrf_token(), first);
    });

    test("current_action returns a string outside a request", fn() {
        assert(type(current_action()) == "string");
    });

    test("assigns helpers report nothing captured outside a request", fn() {
        assert(type(assigns()) == "hash");
        assert_null(assign("anything"));
        assert_eq(view_path(), "");
        assert_eq(render_template(), false);
    });

    test("partial/render_partial raise cleanly without views initialized", fn() {
        let raised = false;
        try {
            partial("nonexistent/partial_name_xyz");
        } catch e {
            raised = true;
        }
        assert(raised);
    });
});

describe("HTTP verb test helpers require a test server", fn() {
    test("get raises the documented test-server error", fn() {
        let message = "";
        let raised = false;
        try {
            get("/anywhere");
        } catch e {
            raised = true;
            message = str(e);
        }
        assert(raised);
        assert(message.includes?("Test server is not running"));
    });

    test("head/options/request fail the same way", fn() {
        let raised = false;
        try {
            head("/x");
        } catch e {
            raised = true;
        }
        assert(raised);

        raised = false;
        try {
            options("/x");
        } catch e {
            raised = true;
        }
        assert(raised);

        raised = false;
        try {
            request("PUT", "/x");
        } catch e {
            raised = true;
        }
        assert(raised);
    });
});

describe("Router internals", fn() {
    test("router_match registers a route and returns null", fn() {
        assert_null(router_match("GET", "/spec/match/path", "specs#show"));
    });

    test("router_match arity errors are clean", fn() {
        let raised = false;
        try {
            router_match("GET", "/x");
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("resource enter/exit balance out", fn() {
        assert_null(router_resource_enter("spec_widgets", {}));
        router_resource_exit();
    });

    test("router_live registers a LiveView route", fn() {
        assert_null(router_live("spec_counter", "live#counter"));
    });

    test("cors registers a rule and validates option keys", fn() {
        assert_null(cors("/spec/api/*"));
        assert_null(cors("/spec/strict/*", {"origins": ["https://app.example.com"],
                                          "credentials": true,
                                          "max_age": 600}));
        let raised = false;
        try {
            cors("/spec/bad/*", {"originz": "*"});
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("skip_csrf registers an exemption pattern", fn() {
        assert_null(skip_csrf("/spec/webhooks/*"));
    });
});
