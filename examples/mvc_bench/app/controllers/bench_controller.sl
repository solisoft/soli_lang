def health(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "text/plain"},
        "body": "OK"
    };
}

def hello(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": "{\"message\": \"Hello\"}"
    };
}

// Regression probe: a named-argument call inside the handler. Watch
// soli_vm_handler_demotions_total on /_metrics — if the VM can't execute
// named-argument calls, this whole handler demotes to the tree-walking
// interpreter on its first request.
def format_greeting(name: String = "world", punct: String = "!") -> String {
    return "Hello, " + name + punct;
}

def named(req: Any) -> Any {
    let message = format_greeting(punct: "?", name: "bench");
    return {
        "status": 200,
        "headers": {"Content-Type": "text/plain"},
        "body": message
    };
}

// Compute-bearing probe pair: `compute` runs on the VM in production, while
// `compute_named` does the same work plus ONE named-argument call — which
// demotes the whole handler to the tree-walking interpreter. The throughput
// delta between the two routes is the real cost of an engine demotion on a
// handler that actually computes something.
def checksum(limit: Int) -> Int {
    let total = 0;
    for i in 1..limit {
        total = total + i * 7 % 13;
    }
    return total;
}

def compute(req: Any) -> Any {
    let total = checksum(2000);
    return {
        "status": 200,
        "headers": {"Content-Type": "text/plain"},
        "body": str(total)
    };
}

def compute_named(req: Any) -> Any {
    let message = format_greeting(name: "engine");
    let total = checksum(2000);
    return {
        "status": 200,
        "headers": {"Content-Type": "text/plain"},
        "body": message + " " + str(total)
    };
}
