# Model concerns

Put reusable mixin modules here: `app/models/concerns/publishable.sl` defines
`module Publishable`. Filenames are `snake_case.sl`; module names are
`PascalCase`.

Everything under `app/models/**` (this folder included) is auto-loaded by
`soli serve`. Models `include` a concern by name — no `import`. Do not put
`Model` subclasses here; those stay in `app/models/`.

## Anatomy

```soli
# app/models/concerns/publishable.sl
module Publishable
  included do
    # Replayed as class-body DSL on the host (`validates`, `has_many`, `scope`).
    validates("published_at", { "presence": true })
    scope("published", fn() { this.where({ "status": "published" }) })
  end

  class_methods do
    def published
      this.where("published_at != null")
    end
  end

  def publish
    self.published_at = DateTime.utc()
  end
end
```

```soli
# app/models/post.sl
class Post < Model
  include Publishable
end
```

- `include Name` copies instance methods onto the class; `extend Name` copies
  them as class methods. Multiple names: `include A, B`.
- `included do … end` / `extended do … end` run against the host class.
- `class_methods do … end` become class methods on the includer.
- `def self.included(base)` / `def self.extended(base)` run after the mix-in.
- A class's own methods win over included ones.
- Modules are not instantiated (`new Publishable()` raises).
- There is no `prepend`, and `super` does not walk into the module.

This is separate from file `import` / `export`. Language reference:
[`docs/soli-language.md`](../../../docs/soli-language.md) → Mixin modules /
Concern hooks.

## Do / Don't

| Do | Don't |
|----|-------|
| Share validations, scopes, and instance helpers used by more than one model | Dump a one-off method into a concern "just in case" |
| Name the file after the module (`publishable.sl` → `Publishable`) | Put `class Post < Model` in this folder |
| `include` from the model after auto-load | `import "./concerns/publishable.sl"` from a model or controller |
| Keep each concern one responsibility | Grow a `Common` mega-module |

## Spec location

Concern specs live in `tests/<name>_concern_spec.sl` and exercise a small
host model (or the real model that includes it) against the real DB.

```soli
describe("Publishable") do
  test("publish sets published_at") do
    @post = Post.create({ "title": "x", "body": "y" })
    @post.publish
    @post.save
    assert(@post.published_at)
  end
end
```

## Before you're done

```bash
soli fmt app/models/concerns/publishable.sl app/models/post.sl
soli lint app/models/concerns/publishable.sl app/models/post.sl
soli test tests/publishable_concern_spec.sl
```
