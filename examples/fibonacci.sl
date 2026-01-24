// Fibonacci sequence in Solilang

fn fibonacci(n: Int) -> Int {
    if (n <= 1) {
        return n;
    }
    return fibonacci(n - 1) + fibonacci(n - 2);
}

// Print first 42 Fibonacci numbers
//print("Fibonacci sequence:");
//let i = 0;
//while (i < 42) {
//    print("fib(" + str(i) + ") =", fibonacci(i));
//    i = i + 1;
//}

fn fibonacci_fast(n: Int) -> Int {
      if (n <= 1) { return n; }
      let a = 0;
      let b = 1;
      for (i in range(2, n + 1)) {
          let temp = a + b;
          a = b;
          b = temp;
      }
      return b;
  }

print(fibonacci_fast(42));