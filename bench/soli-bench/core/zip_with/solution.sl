def zip_with(a, b, f) {
    let out = [];
    let n = len(a);
    if len(b) < n {
        n = len(b);
    }
    for i in range(0, n) {
        out.push(f(a[i], b[i]));
    }
    return out;
}
