// Comprehensive String Methods Benchmark
// Tests string methods that work properly

let iterations = 10000;
let test_string = "Hello World Test String 123";
let test_string_long = "The quick brown fox jumps over the lazy dog. This is a longer test string for benchmarking string operations in Soli. Testing various methods!";
let patterns_test = "hello world hello world hello";

// Test upcase
fn bench_upcase() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.upcase();
        i = i + 1;
    }
}

// Test downcase
fn bench_downcase() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.downcase();
        i = i + 1;
    }
}

// Test capitalize
fn bench_capitalize() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.capitalize();
        i = i + 1;
    }
}

// Test swapcase
fn bench_swapcase() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.swapcase();
        i = i + 1;
    }
}

// Test trim
fn bench_trim() {
    let i = 0;
    let s = "   " + test_string + "   ";
    while (i < iterations) {
        let _ = s.trim();
        i = i + 1;
    }
}

// Test lstrip
fn bench_lstrip() {
    let i = 0;
    let s = "   " + test_string;
    while (i < iterations) {
        let _ = s.lstrip();
        i = i + 1;
    }
}

// Test rstrip
fn bench_rstrip() {
    let i = 0;
    let s = test_string + "   ";
    while (i < iterations) {
        let _ = s.rstrip();
        i = i + 1;
    }
}

// Test chomp
fn bench_chomp() {
    let i = 0;
    let s = test_string + "\n";
    while (i < iterations) {
        let _ = s.chomp();
        i = i + 1;
    }
}

// Test replace
fn bench_replace() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.replace("World", "Soli");
        i = i + 1;
    }
}

// Test contains
fn bench_contains() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.contains("World");
        i = i + 1;
    }
}

// Test starts_with
fn bench_starts_with() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.starts_with("Hello");
        i = i + 1;
    }
}

// Test ends_with
fn bench_ends_with() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.ends_with("123");
        i = i + 1;
    }
}

// Test len
fn bench_len() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.len();
        i = i + 1;
    }
}

// Test squeeze
fn bench_squeeze() {
    let i = 0;
    while (i < iterations) {
        let _ = "aaaabbbbcccc".squeeze();
        i = i + 1;
    }
}

// Test gsub
fn bench_gsub() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.gsub("o", "0");
        i = i + 1;
    }
}

// Test tr
fn bench_tr() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.tr("aeiou", "AEIOU");
        i = i + 1;
    }
}

// Test center
fn bench_center() {
    let i = 0;
    while (i < iterations) {
        let _ = "hi".center(10);
        i = i + 1;
    }
}

// Test ljust
fn bench_ljust() {
    let i = 0;
    while (i < iterations) {
        let _ = "hi".ljust(10);
        i = i + 1;
    }
}

// Test rjust
fn bench_rjust() {
    let i = 0;
    while (i < iterations) {
        let _ = "hi".rjust(10);
        i = i + 1;
    }
}

// Test lpad
fn bench_lpad() {
    let i = 0;
    while (i < iterations) {
        let _ = "hi".lpad(10);
        i = i + 1;
    }
}

// Test rpad
fn bench_rpad() {
    let i = 0;
    while (i < iterations) {
        let _ = "hi".rpad(10);
        i = i + 1;
    }
}

// Test chars
fn bench_chars() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.chars();
        i = i + 1;
    }
}

// Test bytes
fn bench_bytes() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.bytes();
        i = i + 1;
    }
}

// Test lines
fn bench_lines() {
    let i = 0;
    let s = "line1\nline2\nline3\nline4\nline5";
    while (i < iterations) {
        let _ = s.lines();
        i = i + 1;
    }
}

// Test reverse
fn bench_reverse() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.reverse();
        i = i + 1;
    }
}

// Test bytesize
fn bench_bytesize() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.bytesize();
        i = i + 1;
    }
}

// Test hex
fn bench_hex() {
    let i = 0;
    while (i < iterations) {
        let _ = "ff".hex();
        i = i + 1;
    }
}

// Test oct
fn bench_oct() {
    let i = 0;
    while (i < iterations) {
        let _ = "77".oct();
        i = i + 1;
    }
}

// Test empty?
fn bench_empty() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.empty?();
        i = i + 1;
    }
}

// Test include?
fn bench_include() {
    let i = 0;
    while (i < iterations) {
        let _ = test_string.include?("World");
        i = i + 1;
    }
}

// Run all benchmarks
bench_upcase();
bench_downcase();
bench_capitalize();
bench_swapcase();
bench_trim();
bench_lstrip();
bench_rstrip();
bench_chomp();
bench_replace();
bench_contains();
bench_starts_with();
bench_ends_with();
bench_len();
bench_squeeze();
bench_gsub();
bench_tr();
bench_center();
bench_ljust();
bench_rjust();
bench_lpad();
bench_rpad();
bench_chars();
bench_bytes();
bench_lines();
bench_reverse();
bench_bytesize();
bench_hex();
bench_oct();
bench_empty();
bench_include();
