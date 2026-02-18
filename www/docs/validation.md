# Input Validation

SoliLang provides a schema-based input validation system with type coercion and validation rules.

## The V Class

Use the `V` class to create validators:

```soli
V.string()   // String validator
V.int()      // Integer validator
V.float()    // Float validator
V.bool()     // Boolean validator
V.array()    // Array validator
V.hash()     // Hash/object validator
```

## Basic Validation

```soli
let schema = {
    "email": V.string().required().email(),
    "password": V.string().required().min_length(8),
    "age": V.int().optional().min(18)
};

let result = validate(req["json"], schema);

if result["valid"]
    let data = result["data"];
    // Use validated and coerced data
    print("Email:", data["email"]);
    print("Age:", data["age"]);  // Already converted to int
else
    // Handle validation errors
    for error in result["errors"]
        print(error["field"], ":", error["message"]);
    end
end
```

## Validator Methods

### Required Fields

```soli
V.string().required()   // Field must be present
V.string().optional()   // Field can be missing (default)
```

### Nullable Values

```soli
V.string().nullable()   // Null is an acceptable value
```

### Default Values

```soli
V.string().default("guest")      // Use default if missing
V.int().default(0)               // Default for numbers
V.bool().default(false)          // Default for booleans
```

### Numeric Constraints

```soli
V.int().min(0)         // Minimum value
V.int().max(100)       // Maximum value
V.float().min(0.0)     // Minimum value
V.float().max(1.0)     // Maximum value
```

### String Constraints

```soli
V.string().min_length(1)           // Minimum characters
V.string().max_length(255)         // Maximum characters
V.string().pattern("^\\d+$")       // Regex pattern
V.string().email()                 // Valid email format
V.string().url()                   // Valid URL format
```

### Enumeration

```soli
V.string().one_of(["admin", "user", "guest"])
V.int().one_of([1, 2, 3])
```

## Nested Objects

Validate complex objects with nested schemas:

```soli
let address_schema = {
    "street": V.string().required(),
    "city": V.string().required(),
    "zip": V.string().pattern("^\\d{5}$")
};

let user_schema = {
    "name": V.string().required(),
    "email": V.string().required().email(),
    "address": V.hash(address_schema).required()
};

let result = validate(req["json"], user_schema);
```

## Arrays

Validate arrays with element schemas:

```soli
// Validate array of strings
let tags_schema = V.array(V.string().required()).required();

// Validate array of objects
let items_schema = V.array(
    V.hash({
        "id": V.int().required(),
        "name": V.string().required()
    })
).required();

// Usage
let result = validate({
    "items": [
        {"id": 1, "name": "Item 1"},
        {"id": 2, "name": "Item 2"}
    ]
}, {
    "items": V.array(
        V.hash({
            "id": V.int().required(),
            "name": V.string().required()
        })
    ).required()
});
```

## Complete Example: User Registration

```soli
// app/controllers/users_controller.sl

fn new(req)
    {
        "status": 200,
        "body": render("users/new.html", {})
    }
end

fn create(req)
    let schema = {
        "username": V.string().required()
            .min_length(3)
            .max_length(20)
            .pattern(r"^[a-zA-Z0-9_]+$"),
        "email": V.string().required().email(),
        "password": V.string().required().min_length(8),
        "confirm_password": V.string().required(),
        "age": V.int().optional().min(13)
    };

    let result = validate(req["json"], schema);

    if !result["valid"]
        return {
            "status": 422,
            "body": json_stringify({
                "errors": result["errors"]
            })
        };
    end

    let data = result["data"];

    // Check password confirmation
    if data["password"] != data["confirm_password"]
        return {
            "status": 422,
            "body": json_stringify({
                "errors": [{
                    "field": "confirm_password",
                    "message": "passwords do not match",
                    "code": "mismatch"
                }]
            })
        };
    end

    // Create user (example)
    let user = create_user(data["username"], data["email"], data["password"]);

    {
        "status": 201,
        "body": json_stringify({"user": user})
    }
end
```

## API Reference

### validate()

```soli
validate(data, schema)
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `data` | Hash | Data to validate (or `null`) |
| `schema` | Hash | Validation rules |

Returns `{ "valid": Bool, "data": Hash, "errors": Array }`

### Error Format

```soli
{
    "field": "email",
    "message": "must be a valid email",
    "code": "invalid_email"
}
```

### Validator Factory Methods

| Method | Returns |
|--------|---------|
| `V.string()` | String validator |
| `V.int()` | Integer validator |
| `V.float()` | Float validator |
| `V.bool()` | Boolean validator |
| `V.array(schema?)` | Array validator |
| `V.hash(schema?)` | Hash validator |

### Chainable Methods

All validators support: `.required()`, `.optional()`, `.nullable()`, `.default(value)`

String validators: `.min_length(n)`, `.max_length(n)`, `.pattern(regex)`, `.email()`, `.url()`

Numeric validators: `.min(n)`, `.max(n)`

All validators: `.one_of([values])`

## Type Coercion

The validation system automatically coerces types:

```soli
let schema = {
    "age": V.int().required(),
    "active": V.bool().required(),
    "score": V.float().required()
};

// Input (strings get converted)
let input = {
    "age": "25",       // -> 25 (int)
    "active": "true",  // -> true (bool)
    "score": "95.5"    // -> 95.5 (float)
};

let result = validate(input, schema);
// result["data"] contains properly typed values
```
