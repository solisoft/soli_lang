// Array iteration - tests for-in loop entry cost (tree-walker used to snapshot the array)
fn create_array(n: Int) -> String[] {
    let arr: String[] = [];
    let i = 0;
    while (i < n) {
        arr.push("element_" + str(i));
        i = i + 1;
    }
    return arr;
}

fn count_long(arr: String[]) -> Int {
    let count = 0;
    for (x in arr) {
        if (x.length() > 9) {
            count = count + 1;
        }
    }
    return count;
}

let arr = create_array(2000);
let total = 0;
let round = 0;
while (round < 20) {
    total = total + count_long(arr);
    round = round + 1;
}
