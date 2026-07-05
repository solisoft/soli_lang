// Utility module - provides helper functions

export def greet(name: String) -> String {
    return "Hello, " + name + "!";
}

export def max(a: Int, b: Int) -> Int {
    if (a > b) {
        return a;
    }
    return b;
}

export def min(a: Int, b: Int) -> Int {
    if (a < b) {
        return a;
    }
    return b;
}

export def clamp(value: Int, low: Int, high: Int) -> Int {
    if (value < low) {
        return low;
    }
    if (value > high) {
        return high;
    }
    return value;
}
