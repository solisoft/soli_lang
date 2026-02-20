// ============================================================================
// Control Flow Test Suite
// ============================================================================

describe("If/Else Statements", fn() {
    test("if true executes block", fn() {
        let result = 0;
        if (true) {
            result = 1;
        }
        assert_eq(result, 1);
    });

    test("if false skips block", fn() {
        let result = 0;
        if (false) {
            result = 1;
        }
        assert_eq(result, 0);
    });

    test("if-else executes else branch", fn() {
        let result = 0;
        if (false) {
            result = 1;
        } else {
            result = 2;
        }
        assert_eq(result, 2);
    });

    test("if-elsif-else chain", fn() {
        let x = 2;
        let result = "";
        if (x == 1) {
            result = "one";
        } elsif (x == 2) {
            result = "two";
        } else {
            result = "other";
        }
        assert_eq(result, "two");
    });

    test("nested if statements", fn() {
        let a = true;
        let b = true;
        let result = 0;
        if (a) {
            if (b) {
                result = 1;
            }
        }
        assert_eq(result, 1);
    });

    test("if with complex condition", fn() {
        let x = 5;
        let y = 10;
        let result = "";
        if (x > 0 && y > 0) {
            result = "both positive";
        }
        assert_eq(result, "both positive");
    });
});

describe("While Loops", fn() {
    test("while loop iterates", fn() {
        let count = 0;
        while (count < 5) {
            count = count + 1;
        }
        assert_eq(count, 5);
    });

    test("while loop with false condition never executes", fn() {
        let executed = false;
        while (false) {
            executed = true;
        }
        assert_not(executed);
    });

    test("while loop with complex condition", fn() {
        let i = 0;
        let sum = 0;
        while (i < 10 && sum < 20) {
            sum = sum + i;
            i = i + 1;
        }
        assert(sum >= 20 || i >= 10);
    });
});

describe("For-In Loops", fn() {
    test("for-in iterates over array", fn() {
        let arr = [1, 2, 3];
        let sum = 0;
        for (x in arr) {
            sum = sum + x;
        }
        assert_eq(sum, 6);
    });

    test("for-in iterates over range", fn() {
        let sum = 0;
        for (i in range(1, 5)) {
            sum = sum + i;
        }
        assert_eq(sum, 10);
    });

    test("for-in with empty array", fn() {
        let count = 0;
        for (x in []) {
            count = count + 1;
        }
        assert_eq(count, 0);
    });

    test("for-in can access loop variable", fn() {
        let result = [];
        for (i in range(0, 3)) {
            result.push(i * 2);
        }
        assert_eq(result[0], 0);
        assert_eq(result[1], 2);
        assert_eq(result[2], 4);
    });

    test("nested for-in loops", fn() {
        let sum = 0;
        for (i in range(0, 3)) {
            for (j in range(0, 3)) {
                sum = sum + 1;
            }
        }
        assert_eq(sum, 9);
    });
});

describe("Postfix Conditionals", fn() {
    test("postfix if executes when true", fn() {
        let result = 0;
        result = 42 if (true);
        assert_eq(result, 42);
    });

    test("postfix if skips when false", fn() {
        let result = 0;
        result = 42 if (false);
        assert_eq(result, 0);
    });

    test("postfix unless executes when false", fn() {
        let result = 0;
        result = 42 unless (false);
        assert_eq(result, 42);
    });

    test("postfix unless skips when true", fn() {
        let result = 0;
        result = 42 unless (true);
        assert_eq(result, 0);
    });

    test("postfix if still works on same line", fn() {
        let x = 0
        x = 10 if true
        assert_eq(x, 10);
        x = 20 if false
        assert_eq(x, 10);
    });

    test("postfix unless still works on same line", fn() {
        let x = 0
        x = 10 unless false
        assert_eq(x, 10);
        x = 20 unless true
        assert_eq(x, 10);
    });

    test("postfix if with function call on same line", fn() {
        let log = []
        log.push("yes") if true
        log.push("no") if false
        assert_eq(len(log), 1);
        assert_eq(log[0], "yes");
    });

    test("postfix if with expression &&parenthesized condition", fn() {
        let x = 0
        x = 42 if (1 + 1 == 2)
        assert_eq(x, 42);
    });
});

// ============================================================================
// Postfix-if same-line boundary: expression followed by block-if on next line
// must NOT be treated as postfix-if.
// ============================================================================

describe("Expression followed by if/end block", fn() {
    test("simple call then if/end", fn() {
        let log = []
        log.push("a")
        if true
            log.push("b")
        end
        assert_eq(len(log), 2);
        assert_eq(log[0], "a");
        assert_eq(log[1], "b");
    });

    test("call then if/else/end", fn() {
        let log = []
        log.push("before")
        if false
            log.push("then")
        else
            log.push("else")
        end
        assert_eq(len(log), 2);
        assert_eq(log[0], "before");
        assert_eq(log[1], "else");
    });

    test("call then if/elsif/else/end", fn() {
        let x = 2
        let log = []
        log.push("start")
        if x == 1
            log.push("one")
        elsif x == 2
            log.push("two")
        else
            log.push("other")
        end
        assert_eq(len(log), 2);
        assert_eq(log[1], "two");
    });

    test("call then nested if/end blocks", fn() {
        let log = []
        log.push("outer")
        if true
            if true
                log.push("inner")
            end
        end
        assert_eq(len(log), 2);
        assert_eq(log[1], "inner");
    });

    test("call then if/end followed by return value", fn() {
        fn compute()
            let log = []
            log.push("work")
            if true
                log.push("done")
            end
            return log
        end
        let result = compute()
        assert_eq(len(result), 2);
        assert_eq(result[0], "work");
        assert_eq(result[1], "done");
    });

    test("hash-arg method call then if/end", fn() {
        let items = []
        items.push({"key": "a"})
        if true
            items.push({"key": "b"})
        end
        assert_eq(len(items), 2);
        assert_eq(items[0]["key"], "a");
        assert_eq(items[1]["key"], "b");
    });

    test("hash-arg call then if/else/end", fn() {
        let items = []
        items.push({"status": "created"})
        if false
            items.push({"status": "running"})
        else
            items.push({"status": "error"})
        end
        assert_eq(len(items), 2);
        assert_eq(items[1]["status"], "error");
    });

    test("multiple calls then if/end", fn() {
        let log = []
        log.push("one")
        log.push("two")
        log.push("three")
        if true
            log.push("four")
        end
        assert_eq(len(log), 4);
    });

    test("call then for/end loop", fn() {
        let log = []
        log.push("before")
        for i in range(0, 3)
            log.push(str(i))
        end
        assert_eq(len(log), 4);
        assert_eq(log[0], "before");
        assert_eq(log[1], "0");
    });

    test("call then while/end loop", fn() {
        let log = []
        log.push("start")
        let i = 0
        while i < 3
            log.push(str(i))
            i = i + 1
        end
        assert_eq(len(log), 4);
        assert_eq(log[0], "start");
    });

    test("call then if/end inside for loop", fn() {
        let log = []
        for i in range(0, 3)
            log.push("iter")
            if i == 1
                log.push("match")
            end
        end
        assert_eq(len(log), 4);
    });

    test("call then if/end inside if/end", fn() {
        let log = []
        if true
            log.push("outer")
            if true
                log.push("inner")
            end
        end
        assert_eq(len(log), 2);
    });

    test("sequential if/end blocks after calls", fn() {
        let log = []
        log.push("a")
        if true
            log.push("b")
        end
        log.push("c")
        if true
            log.push("d")
        end
        assert_eq(len(log), 4);
        assert_eq(log[0], "a");
        assert_eq(log[1], "b");
        assert_eq(log[2], "c");
        assert_eq(log[3], "d");
    });

    test("complex controller-like pattern", fn() {
        // Simulates the real pattern from Soli Host controllers
        fn update_record(id, data)
            return {"id": id, "data": data}
        end

        let port = 3001
        let result = update_record("vm1", {"next_port": port + 1})
        assert_eq(result["data"]["next_port"], 3002);

        let items = []
        items.push({"app_id": "a1", "role": "primary"})

        if port > 3000
            let r = update_record("a1", {"status": "running"})
            let servers = ["s1", "s2"]
            for s in servers
                let r2 = update_record(s, {"status": "ready"})
                items.push(r2)
            end
        else
            let r = update_record("a1", {"status": "error"})
            items.push(r)
        end

        assert_eq(len(items), 3);
        assert_eq(items[0]["app_id"], "a1");
    });

    test("class method with call then if/end", fn() {
        class Worker {
            fn process(log)
                log.push("processing")
                if len(log) > 0
                    log.push("has items")
                end
                return log
            end
        }
        let w = new Worker()
        let result = w.process([])
        assert_eq(len(result), 2);
        assert_eq(result[0], "processing");
        assert_eq(result[1], "has items");
    });

    test("multiline expression ending before if/end on next line", fn() {
        let data = {
            "name": "test",
            "value": 42
        }
        if data["value"] > 0
            data["value"] = data["value"] + 1
        end
        assert_eq(data["value"], 43);
    });

    test("assignment expression then if/end", fn() {
        let x = 0
        x = 10
        if x > 5
            x = x + 1
        end
        assert_eq(x, 11);
    });

    test("string method call then if/end", fn() {
        let name = "hello"
        let size = len(name)
        if size > 3
            name = name + " world"
        end
        assert_eq(name, "hello world");
    });
});

// ============================================================================
// Real-world MVC controller patterns
// These reproduce actual code from Soli Host controllers that triggered
// the postfix-if parser bug. Each test is self-contained (test scope is isolated).
// ============================================================================

describe("MVC controller patterns", fn() {
    test("create resource with validation &&if/end", fn() {
        let name = "my-app"

        if name == "" ||len(name) < 2
            assert(false, "should not reach here")
        end

        let attrs = {
            "name": name,
            "user_id": "u1",
            "status": "creating",
            "port": 3001
        }

        if attrs["name"] == null
            assert(false, "should not reach here")
        end

        assert_eq(attrs["name"], "my-app");
        assert_eq(attrs["status"], "creating");
    });

    test("update then if/else/end with nested for/end", fn() {
        let app = {
            "name": "test-app",
            "user_id": "u1",
            "status": "creating",
            "port": 3001
        }

        let server = {
            "app_id": "a1",
            "role": "primary",
            "status": "provisioning"
        }

        let success = true
        if success
            app["status"] = "running"
            let servers = ["s1", "s2", "s3"]
            for s in servers
                let rec = {
                    "server_name": s,
                    "status": "ready"
                }
            end
        else
            app["status"] = "error"
        end

        assert_eq(app["status"], "running");
    });

    test("deploy pattern: create record, branch, update status", fn() {
        let app = {"name": "deploy-test", "status": "running"}
        let dep = {"status": "building", "build_log": ""}

        let exit_code = 0
        let workers = ["w1", "w2"]
        for worker in workers
            // simulate ssh_exec on each worker
            let worker_result = "ok"
        end

        if exit_code == 0
            dep["status"] = "live"
        else
            dep["status"] = "failed"
            dep["build_log"] = "error output"
        end

        assert_eq(dep["status"], "live");
    });

    test("scale up pattern: if/elsif with nested loops", fn() {
        let desired_workers = 3
        let current_count = 1
        let scaled = []

        if desired_workers > current_count
            let to_add = desired_workers - current_count
            for i in range(0, to_add)
                let server_name = "worker-" + str(current_count + i + 1)
                let vm_result = {
                    "name": server_name,
                    "status": "provisioning",
                    "type": "worker",
                    "valid": true
                }

                if vm_result["valid"]
                    let server_rec = {
                        "vm_id": "v1",
                        "role": "worker",
                        "status": "provisioning",
                        "port": 3000
                    }
                    scaled.push(server_name)
                end
            end
        elsif desired_workers < current_count
            let to_remove = current_count - desired_workers
            for i in range(0, to_remove)
                scaled.push("removed-" + str(i))
            end
        end

        assert_eq(len(scaled), 2);
        assert_eq(scaled[0], "worker-2");
        assert_eq(scaled[1], "worker-3");
    });

    test("destroy pattern: delete related records in loops", fn() {
        let records = {
            "a1": {"name": "doomed-app", "status": "running"},
            "s1": {"role": "primary", "status": "ready"},
            "s2": {"role": "worker", "status": "ready"},
            "d1": {"type": "deployment", "status": "live"},
            "d2": {"type": "deployment", "status": "failed"}
        }

        // Destroy: delete workers
        let workers = ["s1", "s2"]
        for worker in workers
            records[worker] = null
        end

        // Delete deployments
        let deps = ["d1", "d2"]
        for dep in deps
            records[dep] = null
        end

        // Delete app record
        records["a1"] = null

        assert_eq(records["a1"], null);
        assert_eq(records["s1"], null);
        assert_eq(records["d1"], null);
    });

    test("settings update pattern: conditional updates with hash", fn() {
        let user = {
            "name": "Alice",
            "email": "alice@test.com",
            "password_hash": "hashed123"
        }

        let params = {"name": "Alice B", "email": ""}
        let updates = {}

        if params["name"] != null &&params["name"] != ""
            updates["name"] = params["name"]
        end
        if params["email"] != null &&params["email"] != ""
            updates["email"] = params["email"]
        end

        if len(updates) > 0
            for key in updates.keys()
                user[key] = updates[key]
            end
        end

        assert_eq(user["name"], "Alice B");
        assert_eq(user["email"], "alice@test.com");
    });

    test("SSH key pattern: create with external call then if/end", fn() {
        let name = "my-key"
        let public_key = "ssh-rsa AAAA..."

        if name == "" ||public_key == ""
            assert(false, "should not reach here")
        end

        // Simulate Hetzner API call
        let hetzner_result = {"id": 42, "fingerprint": "ab:cd:ef"}
        let hetzner_key_id = ""
        let fingerprint = ""
        if hetzner_result["error"] == null
            hetzner_key_id = str(hetzner_result["id"])
            fingerprint = hetzner_result["fingerprint"]
        end

        let key_record = {
            "user_id": "u1",
            "name": name,
            "public_key": public_key,
            "fingerprint": fingerprint,
            "hetzner_key_id": hetzner_key_id
        }

        assert_eq(key_record["name"], "my-key");
        assert_eq(key_record["fingerprint"], "ab:cd:ef");
        assert_eq(key_record["hetzner_key_id"], "42");
    });

    test("API status poll pattern: check &&update server status", fn() {
        let server = {
            "app_id": "a1",
            "role": "primary",
            "status": "provisioning",
            "vm_id": "v1"
        }
        let vm = {"ip": "10.0.0.1", "status": "provisioning"}

        // Simulate provisioning check
        let is_ready = true
        if server["status"] == "provisioning"
            if is_ready
                server["status"] = "ready"
                vm["status"] = "ready"
            end
        end

        assert_eq(server["status"], "ready");
        assert_eq(vm["status"], "ready");
    });

    test("auth pattern: validate, query, verify, session", fn() {
        let users = [
            {"id": "1", "email": "user@test.com", "password_hash": "secret"}
        ]

        let email = "user@test.com"
        let password = "secret"

        if email == "" ||password == ""
            assert(false, "should not reach here")
        end

        // Find user by email
        let found = null
        for u in users
            if u["email"] == email
                found = u
            end
        end

        if found == null
            assert(false, "user not found")
        end

        if password != found["password_hash"]
            assert(false, "password mismatch")
        end

        // Simulate session
        let session = {}
        session["user_id"] = found["id"]
        assert_eq(session["user_id"], "1");
    });

    test("register pattern: validate, check duplicate, create, session", fn() {
        let name = "Bob"
        let email = "bob@test.com"
        let password = "secret123"
        let password_confirm = "secret123"

        if name == "" ||email == "" ||password == ""
            assert(false, "fields required")
        end

        if password != password_confirm
            assert(false, "passwords mismatch")
        end

        if len(password) < 8
            assert(false, "password too short")
        end

        let existing = []
        if len(existing) > 0
            assert(false, "already exists")
        end

        let result = {
            "valid": true,
            "record": {
                "id": "1",
                "name": name,
                "email": email,
                "password_hash": "hashed_" + password
            }
        }

        if result["valid"]
            let session = {}
            session["user_id"] = result["record"]["id"]
            assert_eq(session["user_id"], "1");
        else
            assert(false, "creation failed")
        end
    });

    test("proxy config update after app changes", fn() {
        let app = {
            "name": "web",
            "domain": "web.solihost.dev",
            "vm_id": "v1",
            "user_id": "u1",
            "status": "running",
            "port": 3001
        }

        let params = {"domain": "custom.example.com"}
        let updates = {}
        if params["domain"] != null
            updates["domain"] = params["domain"]
        end

        for key in updates.keys()
            app[key] = updates[key]
        end

        // After domain change, update proxy config
        if updates["domain"] != null
            let vm_status = "ready"
            if vm_status == "ready"
                let conf_lines = []
                let apps = [app]
                for a in apps
                    conf_lines.push(a["domain"] + " -> localhost:" + str(a["port"]))
                end
                assert_eq(len(conf_lines), 1);
                assert_eq(conf_lines[0], "custom.example.com -> localhost:3001");
            end
        end

        assert_eq(app["domain"], "custom.example.com");
    });

    test("restart pattern: primary + workers loop", fn() {
        let actions = []

        let app_name = "my-app"
        actions.push("restart:" + app_name)

        let workers = [
            {"vm_ip": "10.0.0.2", "status": "ready"},
            {"vm_ip": "10.0.0.3", "status": "ready"},
            {"vm_ip": "10.0.0.4", "status": "error"}
        ]
        for worker in workers
            if worker["status"] == "ready"
                actions.push("restart-worker:" + worker["vm_ip"])
            end
        end

        assert_eq(len(actions), 3);
        assert_eq(actions[0], "restart:my-app");
        assert_eq(actions[1], "restart-worker:10.0.0.2");
        assert_eq(actions[2], "restart-worker:10.0.0.3");
    });

    test("stop &&update status pattern", fn() {
        let app = {
            "name": "stopping-app",
            "user_id": "u1",
            "status": "running"
        }

        // Simulate ssh_exec stop
        let actions = []
        actions.push("stop:" + app["name"])
        app["status"] = "stopped"

        assert_eq(app["status"], "stopped");
        assert_eq(len(actions), 1);
        assert_eq(actions[0], "stop:stopping-app");
    });

    test("full create flow: validate, provision VM, assign port, create app, create server", fn() {
        let name = "full-test"
        let region = "eu-central"

        // Validate
        if name == "" ||len(name) < 2
            assert(false, "name too short")
        end

        // Simulate VM provisioning
        let vm = {
            "id": "v1",
            "name": "vm-" + region,
            "status": "ready",
            "ip_address": "10.0.0.1",
            "region": region,
            "next_port": 3001
        }

        // Assign port
        let port = vm["next_port"]
        vm["next_port"] = port + 1

        // Create app
        let domain = name + ".solihost.dev"
        let app = {
            "id": "a1",
            "name": name,
            "user_id": "u1",
            "status": "creating",
            "domain": domain,
            "vm_id": vm["id"],
            "port": port
        }

        // Create primary server
        let server = {
            "app_id": app["id"],
            "vm_id": vm["id"],
            "role": "primary",
            "status": "provisioning",
            "port": port
        }

        // Provision if VM ready
        if vm["status"] == "ready"
            let exit_code = 0
            if exit_code == 0
                app["status"] = "running"

                // Update all servers to ready
                let servers = [server]
                for s in servers
                    s["status"] = "ready"
                end

                // Generate proxy config
                let conf = name + ".solihost.dev -> localhost:" + str(port)
                assert_eq(conf, "full-test.solihost.dev -> localhost:3001");
            else
                app["status"] = "error"
            end
        end

        assert_eq(app["status"], "running");
        assert_eq(app["domain"], "full-test.solihost.dev");
        assert_eq(vm["next_port"], 3002);
        assert_eq(server["status"], "ready");
    });
});
