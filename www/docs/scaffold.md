# Scaffold Generator

SoliLang includes a scaffold generator that quickly creates a complete MVC resource including models, controllers, views, tests, and migrations.

## Native / mobile generators

| Command | Creates |
|---------|---------|
| `soli generate devices` | Device model, `POST /devices`, prune helpers — [docs](/docs/native/devices) |
| `soli generate client <platform>` | WebView shells (android, ios, linux, windows; `--fcm` for Android) — [docs](/docs/native/clients) |
| `soli generate app_links` | Well-known deep-link proof routes — [docs](/docs/native/deep-links) |
| `soli generate offline` | Outbox sync push/pull + `soli_outbox.js` — [docs](/docs/native/offline) |

Also: `soli generate auth`, `oidc_provider`, `mailer`, `component`, and `soli desktop build` for local
desktop products.

## Basic Usage

Pass the **singular** resource name (the model name). The generator derives the
plural collection, view directory, and routes from it.

```bash
soli generate scaffold <singular-name>
```

Example:

```bash
soli generate scaffold post
```

This creates:
- Model: `app/models/post_model.sl` (`class Post < Model`)
- Controller: `app/controllers/post_controller.sl` (`class PostController`)
- Views: `app/views/posts/` (index, show, new, edit, `_form` partial)
- Spec: `tests/controllers/post_controller_spec.sl` (controller E2E only — no model test file)
- Migration: `db/migrations/<unix>create_posts_<unix>.sl`
- Routes: CRUD paths appended to `config/routes.sl`

## Generate with Fields

Specify fields with `name:type` syntax:

```bash
soli generate scaffold post title:string body:text author:string
```

### Supported Field Types

| Type | Description |
|------|-------------|
| `string` | Short text field |
| `text` | Long text field |
| `email` | Email address (creates unique index) |
| `password` | Password field (creates unique index) |
| `integer` | Whole number |
| `float` | Decimal number |
| `boolean` | True/false value |
| `date` | Date field |
| `datetime` | Date and time field |
| `url` | URL field |

### Automatic Validations

Fields with types `string`, `text`, `email`, `password`, and `url` automatically get `presence: true` validation.

## Generated Files

### Model

The model includes:
- Field comments documenting the schema
- Auto-generated validations for string-based fields
- Before save callback hooks

```soli
# Post model - auto-generated scaffold
# Collection: posts

class Post < Model
  # Fields
  # title (string)
  # body (text)

  # Validations
  validates("title", { "presence": true })
  validates("body", { "presence": true })

  # Callbacks
  before_save("normalize_fields")
end
```

### Controller

Standard CRUD actions (abbreviated — generated code also has `new` / `edit`
and a `permit()`-based `_permit_params`):

```soli
class PostController < Controller
  static {
    this.layout = "application"
  }

  def index
    posts = Post.all
    render("posts/index", { "posts": posts, "title": "PostController" })
  end

  def show
    post = Post.find(params["id"])
    render("posts/show", { "post": post, "title": "View Post" })
  end

  def create
    permitted = this._permit_params(params)
    post = Post.create(permitted)
    if post._errors
      return render("posts/new", { "post": post, "title": "New Post" })
    end
    return redirect("/posts")
  end

  def _permit_params(params)
    return permit(params, {
      "title": true,
      "body": true
    })
  end
end
```

| Action | Method | Path | Description |
|--------|--------|------|-------------|
| index | GET | /posts | List all records |
| show | GET | /posts/:id | Show single record |
| new | GET | /posts/new | Show create form |
| create | POST | /posts | Create new record |
| edit | GET | /posts/:id/edit | Show edit form |
| update | PUT | /posts/:id | Update record |
| delete | DELETE | /posts/:id | Delete record |

### Views

Located in `app/views/<resource>/`:

| File | Purpose |
|------|---------|
| `index.html.slv` | Table view of all records |
| `show.html.slv` | Detail view of single record |
| `new.html.slv` | Create form |
| `edit.html.slv` | Edit form |
| `_form.html.slv` | Shared partial used by new/edit |

### Tests

Scaffold writes **one** controller E2E spec at
`tests/controllers/<name>_controller_spec.sl`. It covers index / new / show /
edit status codes and a couple of create/update paths. There is no separate
model test file — add `tests/<name>_model_spec.sl` (or similar) yourself if you
want model-level coverage.

### Migration

Migrations create the collection and indexes (email/password fields get a
unique index automatically):

```soli
def up(db)
  db.create_collection("posts")
  # No indexes defined  # unless an email/password field was passed
end

def down(db)
  db.drop_collection("posts")
end
```

## Generating in a Project

Generate scaffolds in your project directory:

```bash
cd my_project
soli generate scaffold post title:string content:text author:string
```

## Field Input Types

The generated form automatically uses appropriate HTML input types:

| Field Type | HTML Input |
|------------|------------|
| string | text |
| text | text |
| email | email |
| password | password |
| integer | number |
| float | number |
| boolean | checkbox |
| date | date |
| datetime | datetime-local |

## Next Steps

After generating a scaffold:

1. Review and customize the model validations
2. Modify the controller logic as needed
3. Style the views to match your application
4. Run migrations with `soli db:migrate up`
5. Populate sample data in `db/seeds.sl` and run `soli db:seed`
6. Start the server and test the CRUD operations
