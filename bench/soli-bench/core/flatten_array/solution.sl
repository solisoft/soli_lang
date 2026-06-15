def flatten(arr) {
    let out = [];
    for item in arr {
        if item.is_a?("array") {
            for sub in flatten(item) {
                out.push(sub);
            }
        } else {
            out.push(item);
        }
    }
    return out;
}
