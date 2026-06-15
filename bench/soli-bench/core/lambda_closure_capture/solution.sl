def make_counter(start) {
    let count = [start];
    return fn() {
        let current = count[0];
        count[0] = current + 1;
        return current;
    };
}
