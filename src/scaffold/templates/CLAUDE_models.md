# Models

`class Xxx < Model` in `app/models/xxx.sl`. CRUD comes from inheritance — don't redefine it.

## Inherited (don't override)

`Xxx.all()`, `Xxx.find(id)`, `Xxx.find_by(field, val)`, `Xxx.where({...})`, `Xxx.create({...})`, `instance.save()`, `instance.delete()`, `Xxx.count()`. Chain `.where(...).order(...).limit(...).offset(...).all()`.

## DSL declared in the class body

```soli
class Post < Model
  belongs_to("user")
  has_many("comments")

  validates("title", { "presence": true, "min_length": 3 })
  before_save("normalize_title")

  scope("published", fn() { this.where({ "status": "published" }) })

  def normalize_title
    this.title = this.title.trim()
  end
end
```

For DSL closures (`scope`, validators, callbacks) prefer `fn() { this.method(...) }` — explicit `this.` over implicit-self.

## Querying safely

- **Hash form** is safe for user input — keys are validated, values bound:
  ```soli
  User.where({ "role": params["role"] })
  ```
- **String form** is developer-trusted only — never concatenate user input into it. Use placeholders:
  ```soli
  User.where("doc.role == @r", { "r": params["role"] })
  ```
- **Raw query** (when the ORM doesn't fit) — use `@sdbql{...}` with `#{expr}` for parameter binding. `#{}` is bound, NOT interpolated as text.
  ```soli
  let users = @sdbql{
    FOR u IN users FILTER u.age >= #{min_age} RETURN u
  }
  ```

## Validation

`Model.create(...)` and `instance.save()` always return; on failure they populate `_errors`. Check it before redirecting.
