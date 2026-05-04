# Metaprogramming

Soli supports runtime introspection and code generation: `respond_to?`, `send`, `method_missing`, `instance_eval`, `class_eval`, `define_method`, `alias_method`, `inherited`, plus instance variable inspection helpers.

This page focuses on two capabilities that let you extend the language from pure Soli code: **adding methods to primitive types** and **defining custom Model DSL helpers**.

## Class reopening with `define_method` and `alias_method`

You can add methods to any class after it's defined. This works on user-defined classes:

```soli
class Greeter
  fn say
    "hello"
  end
end

Greeter.define_method("shout", fn() { "HELLO!" })
Greeter.alias_method("yell", "shout")

let g = Greeter.new
g.shout    # => "HELLO!"
g.yell     # => "HELLO!"
```

`class_eval` runs a block with `self` and `this` bound to the class. Inside, `define_method` writes to that class:

```soli
Greeter.class_eval do
  define_method("wave", fn() { "👋" })
end
```

## Extending primitive types

The same machinery works on built-in primitive types: `Int`, `Float`, `Bool`, `Decimal`, `String`, `Array`, `Hash`, `Symbol`, `Null`. User-defined methods are stored in a per-type overlay and are checked **before** built-in methods, so you can add new methods or shadow existing ones.

```soli
Int.define_method("doubled", fn() { this * 2 })
Int.define_method("times_n", fn(n) { this * n })

3.doubled        # => 6   (zero-arg auto-invokes without parens)
3.doubled()      # => 6
4.times_n(5)     # => 20

String.define_method("shout", fn() { this + "!!!" })
"hi".shout()     # => "hi!!!"

Array.define_method("second", fn() { this[1] })
[10, 20, 30].second()   # => 20
```

`alias_method` works intra-type:

```soli
String.define_method("yell", fn() { this + "!" })
String.alias_method("shout", "yell")
"ok".shout    # => "ok!"
```

### Precedence

Lookup order on a primitive value:

1. User-defined methods (registered via `define_method` / `class_eval`).
2. Built-in methods (`length`, `upcase`, `map`, `times`, etc.).
3. For `Hash`: literal hash-key fallback (`h.foo` returns the value at key `"foo"`).

So a user method on `Hash` will shadow the literal-key fallback. This matches Ruby's monkey-patching semantics.

### Performance

Dispatch is gated by a single Relaxed atomic load. When no user methods have been registered for any primitive type, every call to `3.foo` short-circuits in one cycle and falls straight through to the built-in match. Registered methods cost a single hashmap lookup on the slow path.

### Threading and worker model

The user-method overlay is `thread_local!` — each worker thread (in `soli serve`) has its own copy. Registration that happens during model/controller load runs once per worker, so the overlays are consistent across workers without needing a global lock.

## Named scopes

`scope` is a class-body DSL — same shape as `validates`, `has_many`, `before_save`. The class is auto-prepended to the call, so user code reads naturally:

```soli
class Post < Model
  scope("published", fn(qb) { qb.where({ "status": "published" }) })
  scope("recent",    fn(qb) { qb.order("created_at", "desc").limit(10) })
end

# Accessing the scope name returns a QueryBuilder:
let posts = Post.published.where({ "author_id": 42 }).all
```

The closure receives a fresh `QueryBuilder` for the model and returns a (possibly refined) `QueryBuilder`. Scopes compose with the rest of the query DSL — `Post.published.recent` chains them.

Scope storage is per-thread (`Rc<Function>` is `!Send` and can't go in the process-global `MODEL_REGISTRY`); each worker registers scopes when it loads its model files.

## Cross-references

- [Models](models.md) — the full Model DSL (`validates`, `has_many`, `before_save`, etc.).
- [Validation](validation.md) — built-in validators and the schema-style `V` API.
- See `tests/builtins/extend_int_methods_spec.sl`, `tests/builtins/extend_string_methods_spec.sl`, `tests/builtins/extend_array_hash_spec.sl`, `tests/builtins/extend_primitive_alias_spec.sl`, and `tests/builtins/model_scope_spec.sl` for working examples.
