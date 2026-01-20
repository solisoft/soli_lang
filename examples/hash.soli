// Hash Example - Ruby-style hashes in Solilang

print("=== Hash Basics ===");

// Create a hash with hash rocket syntax
let person = {
    "name" => "Alice",
    "age" => 30,
    "city" => "New York"
};

print("Person:", person);
print("Name:", person["name"]);
print("Age:", person["age"]);

// Empty hash
let empty = {};
print("Empty hash:", empty);
print("Empty hash length:", len(empty));

// Modify hash
person["email"] = "alice@example.com";
print("After adding email:", person);

// Update existing key
person["age"] = 31;
print("After birthday:", person);

print("");
print("=== Hash Functions ===");

let scores = {
    "Alice" => 95,
    "Bob" => 87,
    "Charlie" => 92
};

print("Scores:", scores);
print("Keys:", keys(scores));
print("Values:", values(scores));
print("Length:", len(scores));

print("");
print("=== has_key and delete ===");

print("Has Alice?", has_key(scores, "Alice"));
print("Has David?", has_key(scores, "David"));

let deleted = delete(scores, "Bob");
print("Deleted Bob's score:", deleted);
print("After delete:", scores);

print("");
print("=== merge ===");

let hash1 = {"a" => 1, "b" => 2};
let hash2 = {"b" => 3, "c" => 4};
let merged = merge(hash1, hash2);
print("Hash1:", hash1);
print("Hash2:", hash2);
print("Merged:", merged);

print("");
print("=== entries ===");

let colors = {"red" => "#FF0000", "green" => "#00FF00", "blue" => "#0000FF"};
print("Colors:", colors);
print("Entries:", entries(colors));

print("");
print("=== Iteration ===");

let items = {"apple" => 1.50, "banana" => 0.75, "orange" => 2.00};
print("Shopping list:");
for (pair in entries(items)) {
    print("  -", pair[0], "costs $" + str(pair[1]));
}

print("");
print("=== Numeric Keys ===");

let lookup = {
    1 => "one",
    2 => "two",
    3 => "three"
};
print("Lookup:", lookup);
print("lookup[2] =", lookup[2]);

print("");
print("=== clear ===");

let temp = {"x" => 1, "y" => 2};
print("Before clear:", temp);
clear(temp);
print("After clear:", temp);

print("");
print("=== Pipeline with Hash ===");

fn get_keys(h: Any) -> Any {
    return keys(h);
}

fn first(arr: Any) -> Any {
    return arr[0];
}

let data = {"first" => 100, "second" => 200};
let first_key = data |> get_keys() |> first();
print("First key:", first_key);

print("");
print("Hash examples complete!");
