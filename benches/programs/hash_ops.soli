// Hash operations - tests hash creation, access, and iteration
fn create_hash(n: Int) -> Hash {
    let h = {};
    let i = 0;
    while (i < n) {
        h["key" + str(i)] = i * 2;
        i = i + 1;
    }
    return h;
}

fn hash_sum(h: Hash) -> Int {
    let total = 0;
    let vals = values(h);
    for (v in vals) {
        total = total + v;
    }
    return total;
}

let h = create_hash(500);
let result = hash_sum(h);
