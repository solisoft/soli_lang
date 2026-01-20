// Pipeline operator demonstration in Solilang

fn double(x: Int) -> Int {
    return x * 2;
}

fn addOne(x: Int) -> Int {
    return x + 1;
}

fn square(x: Int) -> Int {
    return x * x;
}

// Pipeline: 5 |> double() |> addOne() = (5 * 2) + 1 = 11
let result1 = 5 |> double() |> addOne();
print("5 |> double() |> addOne() =", result1);

// More complex pipeline
let result2 = 3 |> double() |> square() |> addOne();
print("3 |> double() |> square() |> addOne() =", result2);

// Pipeline with multiple arguments
fn add(x: Int, y: Int) -> Int {
    return x + y;
}

fn multiply(x: Int, y: Int) -> Int {
    return x * y;
}

// 5 |> add(3) = add(5, 3) = 8
let result3 = 5 |> add(3);
print("5 |> add(3) =", result3);

// Chained: 5 |> add(3) |> multiply(2) = (5 + 3) * 2 = 16
let result4 = 5 |> add(3) |> multiply(2);
print("5 |> add(3) |> multiply(2) =", result4);
