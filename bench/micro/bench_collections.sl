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

let N = 10000;

print("=== Array ===");

bench("map", N, fn() {
    let a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    a.map(fn(x) x * 2);
});

bench("filter", N, fn() {
    let a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    a.filter(fn(x) x > 5);
});

bench("reduce", N, fn() {
    let a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    a.reduce(fn(acc, x) { acc + x; }, 0);
});

bench("sort", N, fn() {
    let a = [5, 3, 8, 1, 9, 2, 7, 4, 6, 10];
    a.sort();
});

bench("each", N, fn() {
    let a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let s = 0;
    a.each(fn(x) { s = s + x; });
});

bench("join", N, fn() {
    let a = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    a.join(", ");
});

bench("reverse", N, fn() {
    let a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    a.reverse();
});

bench("uniq", N, fn() {
    let a = [1, 1, 2, 2, 3, 3, 4, 4, 5, 5];
    a.uniq();
});

bench("flatten", N, fn() {
    let a = [[1, 2], [3, 4], [5, 6], [7, 8], [9, 10]];
    a.flatten();
});

bench("push/pop", N, fn() {
    let a = [1, 2, 3, 4, 5];
    a.push(6);
    a.pop();
});

print("");
print("=== Hash ===");

bench("get", N, fn() {
    let h = {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5};
    h.get("c");
});

bench("set", N, fn() {
    let h = {"a": 1, "b": 2, "c": 3};
    h["d"] = 4;
});

bench("keys", N, fn() {
    let h = {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5};
    h.keys();
});

bench("values", N, fn() {
    let h = {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5};
    h.values();
});

bench("merge", N, fn() {
    let h1 = {"a": 1, "b": 2, "c": 3};
    let h2 = {"d": 4, "e": 5, "f": 6};
    h1.merge(h2);
});

bench("has_key", N, fn() {
    let h = {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5};
    h.has_key("c");
});

bench("delete", N, fn() {
    let h = {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5};
    h.delete("c");
});

bench("entries", N, fn() {
    let h = {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5};
    h.entries();
});

bench("invert", N, fn() {
    let h = {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5};
    h.invert();
});

bench("compact", N, fn() {
    let h = {"a": 1, "b": null, "c": 3, "d": null, "e": 5};
    h.compact();
});

print("");
print("=== String ===");

bench("length", N, fn() {
    let s = "hello, world!";
    s.length();
});

bench("upcase", N, fn() {
    let s = "hello, world!";
    s.upcase();
});

bench("downcase", N, fn() {
    let s = "HELLO, WORLD!";
    s.downcase();
});

bench("reverse", N, fn() {
    let s = "hello, world!";
    s.reverse();
});

bench("split", N, fn() {
    let s = "a,b,c,d,e,f,g,h,i,j";
    s.split(",");
});

bench("replace", N, fn() {
    let s = "hello, world!";
    s.replace("world", "soli");
});

bench("trim", N, fn() {
    let s = "  hello, world!  ";
    s.trim();
});

bench("contains", N, fn() {
    let s = "hello, world!";
    s.contains("world");
});

bench("starts_with?", N, fn() {
    let s = "hello, world!";
    s.starts_with?("hello");
});

bench("ends_with?", N, fn() {
    let s = "hello, world!";
    s.ends_with?("world!");
});

bench("concat", N, fn() {
    let s = "hello";
    s + ", world!";
});
