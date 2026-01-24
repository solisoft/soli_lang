// Iterative fibonacci - tests loop performance
fn fib(n: Int) -> Int {
    if (n <= 1) {
        return n;
    }
    let a = 0;
    let b = 1;
    let i = 2;
    while (i <= n) {
        let temp = a + b;
        a = b;
        b = temp;
        i = i + 1;
    }
    return b;
}

let result = fib(30);
