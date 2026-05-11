fn ping(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": "{\"pong\":true}"
    };
}

fn echo_path(req: Any) -> Any {
    return {
        "status": 200,
        "body": req["path"]
    };
}

fn add(req: Any) -> Any {
    let a = (req["query"]["a"] || "0").to_i();
    let b = (req["query"]["b"] || "0").to_i();
    return {
        "status": 200,
        "body": str(a + b)
    };
}

fn echo_method(req: Any) -> Any {
    return {
        "status": 200,
        "body": req["method"]
    };
}

fn echo_header(req: Any) -> Any {
    let name = req["query"]["name"] || "user-agent";
    let val = req["headers"][name] || "";
    return {
        "status": 200,
        "body": str(val)
    };
}

fn json_body(req: Any) -> Any {
    let payload = req["json"];
    let result = {
        "got_name": payload["name"] || "",
        "got_age": payload["age"] || 0
    };
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify(result)
    };
}

fn redirect_demo(req: Any) -> Any {
    return redirect("/ping");
}

fn server_error(req: Any) -> Any {
    return {
        "status": 500,
        "body": "boom"
    };
}

fn array_ops(req: Any) -> Any {
    let nums = [1, 2, 3, 4, 5];
    let doubled = nums.map(fn(n) n * 2);
    let sum = doubled.reduce(fn(acc, n) acc + n, 0);
    return {
        "status": 200,
        "body": str(sum)
    };
}

fn string_ops(req: Any) -> Any {
    let name = req["query"]["name"] || "world";
    return {
        "status": 200,
        "body": ("Hello, " + name + "!").upcase()
    };
}

fn pattern_match(req: Any) -> Any {
    let n = (req["query"]["n"] || "0").to_i();
    let label = match n {
        0 => "zero",
        1 => "one",
        n if n < 0 => "negative",
        n if n > 100 => "large",
        _ => "other"
    };
    return {"status": 200, "body": label};
}

fn try_catch_demo(req: Any) -> Any {
    let result = "";
    try {
        if (req["query"]["fail"]) {
            throw "intentional";
        }
        result = "ok";
    } catch (e) {
        result = "caught:" + str(e);
    }
    return {"status": 200, "body": result};
}

fn pipeline_demo(req: Any) -> Any {
    let nums = [1, 2, 3, 4, 5];
    let total = nums
        .filter(fn(n) n > 1)
        .map(fn(n) n * n)
        .reduce(fn(acc, n) acc + n, 0);
    return {"status": 200, "body": str(total)};
}

fn hash_query(req: Any) -> Any {
    let h = {"a": 1, "b": 2, "c": 3};
    let keys = h.keys();
    return {"status": 200, "body": keys.join(",")};
}

fn for_loop_demo(req: Any) -> Any {
    let total = 0;
    for n in [10, 20, 30] {
        total = total + n;
    }
    return {"status": 200, "body": str(total)};
}

fn while_loop_demo(req: Any) -> Any {
    let i = 0;
    let total = 0;
    while (i < 5) {
        total = total + i;
        i = i + 1;
    }
    return {"status": 200, "body": str(total)};
}

fn closure_demo(req: Any) -> Any {
    let make_adder = fn(x) {
        return fn(y) { return x + y; };
    };
    let add5 = make_adder(5);
    return {"status": 200, "body": str(add5(7))};
}

# Stub action so the `name: "about"` route in routes.sl has a target.
fn about_stub(req: Any) -> Any {
    return {"status": 200, "body": "about"};
}

# Probe action that exercises every auto-generated named-route helper —
# the ones from `resources("posts")` plus the `name: "about"` one-off.
# Returns a JSON object the e2e test asserts on field-by-field; the keys
# match the helper names so failures are easy to read.
fn named_routes_probe(req: Any) -> Any {
    let result = {
        "posts_path": posts_path(),
        "new_post_path": new_post_path(),
        "post_path": post_path(1),
        "edit_post_path": edit_post_path(1),
        "about_path": about_path(),
        "posts_url": posts_url(),
        "post_url": post_url(1),
        "about_url": about_url()
    };
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify(result)
    };
}
