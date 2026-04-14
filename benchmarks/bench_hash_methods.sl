fn bench(label, iterations, block) {
    let start = clock();
    let i = 0;
    while i < iterations {
        block();
        i = i + 1;
    }
    let elapsed = clock() - start;
    let per_iter = elapsed / iterations * 1000000;
    print(label + ": " + str(elapsed) + "s (" + str(per_iter) + " µs/iter)");
}

fn build_hash(size, offset) {
    let h = {};
    let i = 0;
    while i < size {
        h.set("key_" + str(i + offset), i + offset);
        i = i + 1;
    }
    return h;
}

fn build_sparse_hash(size) {
    let h = {};
    let i = 0;
    while i < size {
        if i % 4 == 0
            h.set("key_" + str(i), null);
        else
            h.set("key_" + str(i), i);
        end
        i = i + 1;
    }
    return h;
}

let SIZE = 1024;
let READ_N = 50000;
let CLONE_N = 5000;

let base = build_hash(SIZE, 0);
let other = build_hash(SIZE, SIZE);
let sparse = build_sparse_hash(SIZE);

print("=== Focused Hash Methods ===");
print("size=" + str(SIZE));

bench("get", READ_N, fn() {
    base.get("key_512");
});

bench("set existing", READ_N, fn() {
    base.set("key_512", 42);
});

bench("has_key", READ_N, fn() {
    base.has_key("key_512");
});

bench("delete+restore", READ_N, fn() {
    let old = base.delete("key_512");
    base.set("key_512", old);
});

bench("keys", CLONE_N, fn() {
    base.keys();
});

bench("values", CLONE_N, fn() {
    base.values();
});

bench("entries", CLONE_N, fn() {
    base.entries();
});

bench("merge", CLONE_N, fn() {
    base.merge(other);
});

bench("invert", CLONE_N, fn() {
    base.invert();
});

bench("compact", CLONE_N, fn() {
    sparse.compact();
});
