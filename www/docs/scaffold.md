# Scaffold Generator

SoliLang includes a scaffold generator that quickly creates a complete MVC resource including models, controllers, views, tests, and migrations.

## Basic Usage

Generate a scaffold for a resource:

```bash
soli generate scaffold <name>
```

Example:

```bash
soli generate scaffold users
```

This creates:
- Model: `app/models/users_model.sl`
- Controller: `app/controllers/users_controller.sl`
- Views: `app/views/users/` (index, show, new, edit, _form partial)
- Tests: `tests/models/users_test.sl`, `tests/controllers/users_controller_test.sl`
- Migration: `db/migrations/<timestamp>create_users_<timestamp>.sl`
- Routes: Added to `config/routes.sl`

## Generate with Fields

Specify fields with `name:type` syntax:

```bash
soli generate scaffold users name:string email:text age:integer
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
# Users model - auto-generated scaffold
class Users < Model
    static
        # Fields
        # name (string)
        # email (email)

        # Validations
        validates("name", { "presence": true })
        validates("email", { "presence": true })
    end

    before_save("normalize_fields")
end
```

### Controller

Standard CRUD actions:

```soli
class UsersController < Controller
    def index(req)
        users = Users.all
        render("users/index", { "users": users })
    end

    def show(req)
        user = Users.find(params["id"])
        render("users/show", { "user": user })
    end

    def create(req)
        permitted = this._permit_params(params)
        user = Users.create(permitted)
        if user._errors
            return render("users/new", { "user": user })
        end
        return redirect("/users")
    end

    def update(req)
        id = params["id"]
        permitted = this._permit_params(params)
        Users.update(id, permitted)
        return redirect("/users")
    end

    def delete(req)
        id = params["id"]
        Users.delete(id)
        return redirect("/users")
    end

    def _permit_params(params)
        return {
            "name": params["name"],
            "email": params["email"]
        }
    end
end
```

| Action | Method | Path | Description |
|--------|--------|------|-------------|
| index | GET | /users | List all records |
| show | GET | /users/:id | Show single record |
| new | GET | /users/new | Show create form |
| create | POST | /users | Create new record |
| edit | GET | /users/:id/edit | Show edit form |
| update | PUT | /users/:id | Update record |
| delete | DELETE | /users/:id | Delete record |

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

Model tests include:
- Collection name validation
- Record creation tests
- Find by ID tests
- Validation tests

Controller tests include:
- Index action rendering
- Show action rendering
- New/edit form rendering
- Create/update/delete redirects

### Migration

Migrations create the collection and indexes:

```soli
def up(db)
    db.create_collection("users")
    db.create_index("users", "idx_email", ["email"], { "unique": true })
end

def down(db)
    db.drop_index("users", "idx_email")
    db.drop_collection("users")
end
```

## Generating in a Project

Generate scaffolds in your project directory:

```bash
cd my_project
soli generate scaffold posts title:string content:text author:string
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
5. Start the server and test the CRUD operations
