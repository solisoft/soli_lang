# Self-timed hash get/set — same loop as benches/hash_get_set.rs VM suite.
# Prints milliseconds for the timed loop only (no process startup).

const ITERS = 200000;

def build(n: Int) -> Array {
    let keys = [];
    let h = {};
    let i = 0;
    while i < n {
        let key = "k" + str(i);
        keys.push(key);
        h[key] = i;
        i = i + 1;
    }
    [keys, h]
}

def bench_get_index(n: Int) -> Float {
    let pair = build(n);
    let keys = pair[0];
    let h = pair[1];
    let j = 0;
    let total = 0;
    let start = clock();
    while j < ITERS {
        let k = j % n;
        total = total + h[keys[k]];
        j = j + 1;
    }
    (clock() - start) * 1000
}

def bench_get_method(n: Int) -> Float {
    let pair = build(n);
    let keys = pair[0];
    let h = pair[1];
    let j = 0;
    let total = 0;
    let start = clock();
    while j < ITERS {
        let k = j % n;
        total = total + h.get(keys[k]);
        j = j + 1;
    }
    (clock() - start) * 1000
}

def bench_set_index(n: Int) -> Float {
    let pair = build(n);
    let keys = pair[0];
    let h = pair[1];
    let j = 0;
    let start = clock();
    while j < ITERS {
        let k = j % n;
        h[keys[k]] = j;
        j = j + 1;
    }
    (clock() - start) * 1000
}

def bench_set_method(n: Int) -> Float {
    let pair = build(n);
    let keys = pair[0];
    let h = pair[1];
    let j = 0;
    let start = clock();
    while j < ITERS {
        let k = j % n;
        h.set(keys[k], j);
        j = j + 1;
    }
    (clock() - start) * 1000
}

print("hash_get_set (" + str(ITERS) + " iterations)");
for n in [4, 8, 16, 64, 256] {
    print("  get_index/" + str(n) + ": " + str(bench_get_index(n)) + "ms");
    print("  get_method/" + str(n) + ": " + str(bench_get_method(n)) + "ms");
    print("  set_index/" + str(n) + ": " + str(bench_set_index(n)) + "ms");
    print("  set_method/" + str(n) + ": " + str(bench_set_method(n)) + "ms");
}
