# Real Domain Modeling with Soli: Scopes, Soft Deletes, and Transactions

The Model layer in Soli is deliberately rich in the areas that matter for real applications.

Three features in particular stand out for teams building anything beyond toy CRUD: **named scopes**, **soft deletes**, and **first-class transactions**.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/advanced-modeling.jpg" width="1024" height="576" alt="Professional diagram illustrating Soli model features: soft delete lifecycle, named scopes as composable query filters, and database transactions wrapping operations." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">The modeling tools that keep complex domain logic maintainable over years.</figcaption>
</figure>

## Named Scopes

Scopes let you define reusable, composable query fragments directly on the model:

```soli
class User < Model
    scope("active", fn() {
        this.where("active = @a", { "a": true })
    })

    scope("recent", fn() {
        this.order("created_at", "desc").limit(50)
    })
end
```

Usage is natural and chainable:

```soli
let users = User.active.recent.where("role = @r", { "r": "admin" }).all
```

Scopes compose cleanly and keep query logic out of controllers and views.

## Soft Deletes

`soft_delete` is a one-line declaration with powerful behavior:

```soli
class Post < Model
    soft_delete
end
```

- `post.delete()` sets `deleted_at` instead of removing the row.
- `post.restore()` clears it.
- `Post.all` and normal queries automatically exclude deleted records.
- `Post.with_deleted` and `Post.only_deleted` give you the other two common views.

This pattern is so common in production systems that having it built into the base `Model` class removes an entire category of repetitive and error-prone implementation work.

## Transactions

The `transaction` method supports both block style and manual control:

```soli
User.transaction {
    user = User.create!(data)
    Profile.create!(user_id: user.id, ...)

    # Everything commits together or rolls back
}
```

You also get lower-level control when you need it for complex multi-statement operations.

## The Compound Effect

When you combine scopes + soft deletes + transactions with the rest of the model toolkit (validations, callbacks, relationships, `.similar()`, aggregations), you get a modeling surface that lets you express real business rules at the right layer — without fighting the framework or reaching for raw SQL on every complex query.

These are the features that separate "it works for the happy path" applications from systems that remain maintainable as complexity grows.

---

Soli’s model layer is opinionated in exactly the places that save teams the most time and bugs over a multi-year lifespan. Scopes, soft deletes, and transactions are three of the highest-ROI parts of that bet.