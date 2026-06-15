def piped_steps {
    return range(1, 11) |> double_each;
}

def piped_sum {
    return range(1, 11) |> double_each |> keep_big |> sum_each;
}

def double_each(arr) {
    return arr.map(fn(x) x * 2);
}

def keep_big(arr) {
    return arr.filter(fn(x) x > 5);
}

def sum_each(arr) {
    return arr.sum;
}
