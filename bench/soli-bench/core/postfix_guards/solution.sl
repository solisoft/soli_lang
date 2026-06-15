def greet_adult(age) {
    return age >= 18 ? "hello adult" : "hello minor";
}

def safe_value(s) {
    return s.blank? ? "default" : s;
}

def maybe_double(x) {
    if x == null {
        return null;
    }
    return x * 2;
}
