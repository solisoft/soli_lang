def with_default(x, default) {
    return x ?? default;
}

def coalesce(a, b, c) {
    return a ?? b ?? c;
}

def safe_lookup(hash, key) {
    return hash[key] ?? "missing";
}
