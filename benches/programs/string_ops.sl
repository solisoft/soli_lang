// String operations - tests string concatenation and manipulation
fn build_string(n: Int) -> String {
    let s = "";
    let i = 0;
    while (i < n) {
        s = s + "x";
        i = i + 1;
    }
    return s;
}

fn count_chars(s: String) -> Int {
    return len(s);
}

let s = build_string(500);
let result = count_chars(s);
