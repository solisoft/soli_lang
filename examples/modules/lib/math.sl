// Math module - provides basic math functions

export def add(a: Int, b: Int) -> Int {
    return a + b;
}

export def subtract(a: Int, b: Int) -> Int {
    return a - b;
}

export def multiply(a: Int, b: Int) -> Int {
    return a * b;
}

export def divide(a: Int, b: Int) -> Int {
    return a / b;
}

export def square(n: Int) -> Int {
    return n * n;
}

export def abs(n: Int) -> Int {
    if (n < 0) {
        return -n;
    }
    return n;
}

// Private helper function (not exported)
def internal_helper(x: Int) -> Int {
    return x * 2;
}
