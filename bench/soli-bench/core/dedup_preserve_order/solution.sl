def dedup(arr) {
    let seen = {};
    let out = [];
    for item in arr {
        let key = str(item);
        if !seen.has_key(key) {
            seen[key] = true;
            out.push(item);
        }
    }
    return out;
}
