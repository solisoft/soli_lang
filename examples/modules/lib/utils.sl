// Utility module - provides helper functions

export fn greet(name: String) -> String {
    return "Hello, " + name + "!";
}

export fn max(a: Int, b: Int) -> Int {
    if (a > b) {
        return a;
    }
    return b;
}

export fn min(a: Int, b: Int) -> Int {
    if (a < b) {
        return a;
    }
    return b;
}

export fn clamp(value: Int, low: Int, high: Int) -> Int {
    if (value < low) {
        return low;
    }
    if (value > high) {
        return high;
    }
    return value;
}
