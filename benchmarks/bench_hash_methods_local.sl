fn report(label, iterations, elapsed) {
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

fn bench_get(base, read_n) {
    let i = 0;
    let start = clock();
    while i < read_n {
        base.get("key_512");
        i = i + 1;
    }
    report("get", read_n, clock() - start);
}

fn bench_set(base, read_n) {
    let i = 0;
    let start = clock();
    while i < read_n {
        base.set("key_512", 42);
        i = i + 1;
    }
    report("set existing", read_n, clock() - start);
}

fn bench_has_key(base, read_n) {
    let i = 0;
    let start = clock();
    while i < read_n {
        base.has_key("key_512");
        i = i + 1;
    }
    report("has_key", read_n, clock() - start);
}

fn bench_delete_restore(base, read_n) {
    let i = 0;
    let start = clock();
    while i < read_n {
        let old = base.delete("key_512");
        base.set("key_512", old);
        i = i + 1;
    }
    report("delete+restore", read_n, clock() - start);
}

fn bench_keys(base, clone_n) {
    let i = 0;
    let start = clock();
    while i < clone_n {
        base.keys();
        i = i + 1;
    }
    report("keys", clone_n, clock() - start);
}

fn bench_values(base, clone_n) {
    let i = 0;
    let start = clock();
    while i < clone_n {
        base.values();
        i = i + 1;
    }
    report("values", clone_n, clock() - start);
}

fn bench_entries(base, clone_n) {
    let i = 0;
    let start = clock();
    while i < clone_n {
        base.entries();
        i = i + 1;
    }
    report("entries", clone_n, clock() - start);
}

fn bench_merge(base, other, clone_n) {
    let i = 0;
    let start = clock();
    while i < clone_n {
        base.merge(other);
        i = i + 1;
    }
    report("merge", clone_n, clock() - start);
}

fn bench_invert(base, clone_n) {
    let i = 0;
    let start = clock();
    while i < clone_n {
        base.invert();
        i = i + 1;
    }
    report("invert", clone_n, clock() - start);
}

fn bench_compact(sparse, clone_n) {
    let i = 0;
    let start = clock();
    while i < clone_n {
        sparse.compact();
        i = i + 1;
    }
    report("compact", clone_n, clock() - start);
}

fn run_suite() {
    let size = 1024;
    let read_n = 50000;
    let clone_n = 5000;
    let base = build_hash(size, 0);
    let other = build_hash(size, size);
    let sparse = build_sparse_hash(size);

    print("=== Focused Hash Methods Local ===");
    print("size=" + str(size));
    bench_get(base, read_n);
    bench_set(base, read_n);
    bench_has_key(base, read_n);
    bench_delete_restore(base, read_n);
    bench_keys(base, clone_n);
    bench_values(base, clone_n);
    bench_entries(base, clone_n);
    bench_merge(base, other, clone_n);
    bench_invert(base, clone_n);
    bench_compact(sparse, clone_n);
}

run_suite();
