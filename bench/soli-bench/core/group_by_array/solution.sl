def group_by(arr, key_fn) {
    let groups = {};
    for item in arr {
        let key = str(key_fn(item));
        if !groups.has_key(key) {
            groups[key] = [];
        }
        groups[key].push(item);
    }
    return groups;
}
