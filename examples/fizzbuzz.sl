// FizzBuzz in Solilang

fn fizzbuzz(n: Int) -> String {
    if (n % 15 == 0) {
        return "FizzBuzz";
    }
    if (n % 3 == 0) {
        return "Fizz";
    }
    if (n % 5 == 0) {
        return "Buzz";
    }
    return str(n);
}

print("FizzBuzz 1 to 20:");
let i = 1;
while (i <= 20) {
    print(fizzbuzz(i));
    i = i + 1;
}
