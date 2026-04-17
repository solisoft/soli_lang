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

let N = 100000;

print("=== String Interpolation ===");

bench("single var", N, fn() {
    let name = "Alice";
    let s = "Hello #{name}!";
});

bench("two vars", N, fn() {
    let first = "John";
    let last = "Doe";
    let s = "#{first} #{last}";
});

bench("int var", N, fn() {
    let n = 42;
    let s = "Value: #{n}";
});

bench("float var", N, fn() {
    let f = 3.14159;
    let s = "Pi: #{f}";
});

bench("expression", N, fn() {
    let a = 2;
    let b = 3;
    let s = "Sum is #{a + b}";
});

bench("method call", N, fn() {
    let text = "hello";
    let s = "Upper: #{text.upcase()}";
});

bench("array index", N, fn() {
    let names = ["Alice", "Bob", "Carol"];
    let s = "First: #{names[0]}";
});

bench("hash access", N, fn() {
    let person = {"name": "Charlie", "age": 30};
    let s = "Name: #{person["name"]}";
});

bench("many vars (5)", N, fn() {
    let a = "A";
    let b = "B";
    let c = "C";
    let d = "D";
    let e = "E";
    let s = "#{a}-#{b}-#{c}-#{d}-#{e}";
});

bench("many vars (10)", N, fn() {
    let v1 = 1;
    let v2 = 2;
    let v3 = 3;
    let v4 = 4;
    let v5 = 5;
    let v6 = 6;
    let v7 = 7;
    let v8 = 8;
    let v9 = 9;
    let v10 = 10;
    let s = "#{v1},#{v2},#{v3},#{v4},#{v5},#{v6},#{v7},#{v8},#{v9},#{v10}";
});

bench("mixed types", N, fn() {
    let name = "Alice";
    let age = 30;
    let score = 98.5;
    let active = true;
    let s = "#{name} (#{age}) score=#{score} active=#{active}";
});

bench("long text + vars", N, fn() {
    let user = "Alice";
    let count = 42;
    let s = "Dear #{user}, you have #{count} new messages waiting in your inbox.";
});

bench("concat (baseline)", N, fn() {
    let name = "Alice";
    let s = "Hello " + name + "!";
});

bench("concat many (5)", N, fn() {
    let a = "A";
    let b = "B";
    let c = "C";
    let d = "D";
    let e = "E";
    let s = a + "-" + b + "-" + c + "-" + d + "-" + e;
});
