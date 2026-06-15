def count_by(arr, key_fn) {
    let counts = {};
    for item in arr {
        let key = str(key_fn(item));
        if counts.has_key(key) {
            counts[key] = counts[key] + 1;
        } else {
            counts[key] = 1;
        }
    }
    return counts;
}
