# Server-Side Password Validation in Soli

Frontend password rules are a good start, but they're just a suggestion. Anyone can bypass your browser, curl your API, and register with `"password"` as their password. Server-side validation isn't optional — it's your last line of defense.

Soli's validation system gives you five character-class rules that mirror what password managers and security standards expect, plus a way to tell the browser the same rules through the `passwordrules` HTML attribute.

## Character-Class Rules

```soli
# At least one letter (a-z, A-Z)
V.string().letters()

# At least one uppercase AND one lowercase
V.string().mixed_case()

# At least one digit
V.string().numbers()

# At least one symbol (non-alphanumeric)
V.string().symbols()
```

Each rule produces a clear, specific error message:

```soli
{
  "field": "password",
  "message": "must contain at least one letter",
  "code": "letters"
}
```

You can chain them together to build any policy:

```soli
schema = {
  "password": V.string()
    .required()
    .min_length(8)
    .max_length(64)
    .letters()
    .mixed_case()
    .numbers()
    .symbols()
}
```

## Validation Behaviour

| Rule | Rejects | Accepts |
|------|---------|---------|
| `.letters()` | `"123456!"` | `"abc123!"` |
| `.mixed_case()` | `"alllowercase1!"` | `"MixedCase1!"` |
| `.numbers()` | `"NoDigits!"` | `"HasDigits1"` |
| `.symbols()` | `"NoSymbols1"` | `"HasSymbols!"` |

## Password Confirmation

The `.confirmation("field")` method validates that the field's value matches another field in the same data hash — no manual post-validation boilerplate:

```soli
schema = {
  "password": V.string().required().min_length(8).mixed_case().numbers(),
  "confirm_password": V.string().required().confirmation("password")
}

result = validate(req["json"], schema)
# If password != confirm_password, result["valid"] is false
# with error: { "field": "confirm_password", "message": "does not match", "code": "confirmation" }
```

The `confirm_password` validator declares `confirmation("password")`, which means: my value must equal the value of field `password`. The validation engine handles the comparison automatically during the field validation pass — no separate equality check needed.

## The passwordrules HTML Attribute

Browser password managers (iCloud Keychain, 1Password, Bitwarden, Chrome) respect the `passwordrules` attribute on `<input type="password">`. It tells the generator what kind of password to create — so users never see "your password doesn't match our rules" after auto-generating one.

Generate the attribute string directly from your validator chain:

```soli
let rules = V.string()
  .min_length(12)
  .max_length(64)
  .mixed_case()
  .numbers()
  .symbols()
  .to_password_rules_string()

# → "minlength: 12; maxlength: 64; required: lower; required: upper; required: digit; required: special;"
```

Then use it in your template:

```erb
<input type="password" name="password" required
  minlength="12" maxlength="64"
  passwordrules="<%= to_password_rules_string() %>">
```

The same validator chain enforces the rules server-side and tells the browser about them — no duplication, no drift.

## Full Registration Endpoint

Here's a complete controller action tying it all together:

```soli
# app/controllers/users_controller.sl

fn create(req)
  schema = {
    "username": V.string().required()
      .min_length(3)
      .max_length(20)
      .pattern(r"^[a-zA-Z0-9_]+$"),
    "email": V.string().required().email(),
    "password": V.string().required()
      .min_length(12)
      .max_length(64)
      .mixed_case()
      .numbers()
      .symbols(),
    "confirm_password": V.string().required().confirmation("password")
  }

  result = validate(req["json"], schema)

  if !result["valid"]
    return {"status": 422, "body": json_stringify({"errors": result["errors"]})}
  end

  data = result["data"]

  user = User.create({
    "username": data["username"],
    "email": data["email"],
    "password_hash": Crypto.argon2_hash(data["password"])
  })

  if user["valid"]
    session_regenerate()
    session_set("user_id", user["id"])
    redirect("/dashboard")
  else
    return {"status": 422, "body": json_stringify({"errors": user["errors"]})}
  end
end
```

## Tying It to Registration Templates

In your registration form template, use `to_password_rules_string()` in the view to generate the `passwordrules` attribute:

```erb
<%
  let pw_rules = V.string()
    .min_length(12)
    .max_length(64)
    .mixed_case()
    .numbers()
    .symbols()
    .to_password_rules_string()
%>

<input type="password" name="password" required
  minlength="12" maxlength="64"
  passwordrules="<%= pw_rules %>">

<input type="password" name="confirm_password" required>
```

Now the password manager, the browser's built-in validation, and your server all enforce the same policy from a single source of truth.

## Summary

- `.letters()`, `.mixed_case()`, `.numbers()`, `.symbols()` enforce character-class requirements server-side
- Chain them with `.min_length()` / `.max_length()` for a complete password policy
- Use `.to_password_rules_string()` to generate the `passwordrules` HTML attribute
- Use `.confirmation("field")` to validate that a field matches another field — no post-validation equality check needed
- Keep a single source of truth — the validator chain — for both client hints and server enforcement
