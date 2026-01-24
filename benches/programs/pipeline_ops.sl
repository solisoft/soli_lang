// Pipeline operations - tests pipeline operator overhead
fn double(x: Int) -> Int {
    return x * 2;
}

fn addOne(x: Int) -> Int {
    return x + 1;
}

fn square(x: Int) -> Int {
    return x * x;
}

fn transform(x: Int) -> Int {
    return x |> double() |> addOne() |> square();
}

let total = 0;
let i = 0;
while (i < 1000) {
    total = total + transform(i);
    i = i + 1;
}
let result = total;
