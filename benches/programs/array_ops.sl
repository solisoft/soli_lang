// Array operations - tests array creation, access, and iteration
fn array_sum(arr: Int[]) -> Int {
    let total = 0;
    for (x in arr) {
        total = total + x;
    }
    return total;
}

fn create_array(n: Int) -> Int[] {
    let arr: Int[] = [];
    let i = 0;
    while (i < n) {
        arr.push(i * 2);
        i = i + 1;
    }
    return arr;
}

let arr = create_array(1000);
let result = array_sum(arr);
