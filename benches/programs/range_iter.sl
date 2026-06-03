// Range iteration - tests for-in over ranges (tree-walker used to materialize the
// whole range into an array before looping; the VM iterates inline)
fn sum_range(n: Int) -> Int {
    let total = 0;
    for (i in 0..n) {
        total = total + i;
    }
    return total;
}

let total = 0;
let round = 0;
while (round < 20) {
    total = total + sum_range(50000);
    round = round + 1;
}
