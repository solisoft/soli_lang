# Destructuring Done Right: Rich Pattern Matching in Soli

Many languages treat pattern matching as an advanced, niche feature.

Soli includes a powerful `match` expression as a core part of everyday control flow — with guards, deep destructuring, rest patterns, and type-aware matching.

The result is code that is both more concise and more correct.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/pattern-matching.jpg" width="1024" height="576" alt="Abstract elegant visualization of pattern matching: stylized geometric shapes and data structures being matched and destructured with glowing connections, representing guards, arrays, hashes, and rest patterns in a clean conceptual style." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Pattern matching as a first-class, everyday tool — represented conceptually.</figcaption>
</figure>

## Basic and Guarded Matching

```soli
match status {
    "active" => "User can access the system",
    "pending" => "Awaiting approval",
    "suspended" => "Temporarily disabled",
    _ => "Unknown status"
}
```

Guards make it dramatically more useful:

```soli
match amount {
    n if n < 0 => "negative",
    0 => "zero",
    n if n > 0 && n < 100 => "small positive",
    n if n >= 100 => "large positive"
}
```

HTTP status handling becomes trivial and exhaustive:

```soli
match code {
    code if code >= 200 && code < 300 => "Success",
    404 => "Not Found",
    code if code >= 500 => "Server Error",
    _ => "Client Error"
}
```

## Array and Rest Destructuring

```soli
match items {
    [] => "No items",
    [first] => "Only one item: " + first,
    [first, second, ...rest] => {
        "First two: " + first + ", " + second +
        " (" + len(rest) + " more)"
    }
}
```

This pattern appears constantly when processing query results, API payloads, or configuration arrays.

## Hash Destructuring

```soli
match user {
    {name: n, email: e} => n + " <" + e + ">",
    {name: n} => n + " (no email)",
    _ => "Unknown user shape"
}
```

You can match on structure without worrying about extra keys or missing ones.

## Why This Matters

Pattern matching eliminates an entire category of bugs:

- Forgetting a case (the compiler/runtime can help you be exhaustive)
- Manual `if/elsif` chains that drift out of sync with reality
- Ugly nested conditionals when destructuring complex data

In controllers and service objects especially, it produces code that is easier to read and far easier to modify safely when requirements change.

If you come from a language where pattern matching feels like an academic feature, try using it for a week in real Soli code. You will quickly wonder how you lived without it.

---

Soli’s version strikes an excellent balance: powerful enough to be genuinely useful on real data, but lightweight enough that it doesn’t require a separate type system or heavy syntax to be productive.