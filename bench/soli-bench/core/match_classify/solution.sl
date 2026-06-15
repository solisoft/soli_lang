def classify(x) {
    return match x {
        0 => "zero",
        n if n.is_a?("int") && n > 0 && n < 10 => "positive-small",
        n if n.is_a?("int") && n >= 10 => "positive-large",
        n if n.is_a?("int") && n < 0 => "negative",
        _ => "other"
    };
}
