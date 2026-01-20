// JSON Functions Test
// Tests json_parse and json_stringify

// Test json_stringify
print("=== Testing json_stringify ===");

let obj = {"name" => "Alice", "age" => 30, "active" => true};
let json = json_stringify(obj);
print("Hash to JSON:", json);

let arr = [1, 2, 3, "four", true, null];
let json_arr = json_stringify(arr);
print("Array to JSON:", json_arr);

let nested = {
    "user" => {"name" => "Bob", "id" => 123},
    "tags" => ["admin", "user"],
    "metadata" => {"version" => 1.5}
};
let json_nested = json_stringify(nested);
print("Nested to JSON:", json_nested);

// Test json_parse
print("\n=== Testing json_parse ===");

let parsed = json_parse(json);
print("Parsed hash:", parsed);
print("Name:", parsed["name"]);
print("Age:", parsed["age"]);

let parsed_arr = json_parse(json_arr);
print("Parsed array:", parsed_arr);
print("First element:", parsed_arr[0]);

let parsed_nested = json_parse(json_nested);
print("Parsed nested:", parsed_nested);
print("User name:", parsed_nested["user"]["name"]);
print("First tag:", parsed_nested["tags"][0]);

// Round-trip test
print("\n=== Round-trip test ===");
let original = {"x" => 1, "y" => [2, 3], "z" => {"a" => "b"}};
let round_tripped = json_parse(json_stringify(original));
print("Original:", original);
print("Round-tripped:", round_tripped);

print("\n=== All JSON tests passed! ===");
