// Regression probe: an OOP controller with a zero-argument action. Zero-arg
// class methods are hard-wired to the tree-walking interpreter in
// call_class_method (they never attempt the VM) — this route makes that
// behavior observable in benchmarks.
class ProbeController {
    def zero() -> Any {
        return {
            "status": 200,
            "headers": {"Content-Type": "text/plain"},
            "body": "zero"
        };
    }
}
