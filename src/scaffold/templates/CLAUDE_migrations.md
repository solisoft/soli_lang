# Migrations

Files: `db/migrations/YYYYMMDDHHMMSS_description.sl`. **Generate, never hand-name** — the timestamp prefix is the ordering key.

```bash
soli generate migration create_posts
```

## Shape

```soli
def up
  create_collection("posts")
  add_index("posts", "user_id")
end

def down
  drop_collection("posts")
end
```

## Rules

- Always provide a real `down()` — no `# TODO` placeholder. Every migration must be reversible.
- Run pending: `soli db:migrate up`
- Roll back the last one: `soli db:migrate down`
- Inspect state: `soli db:migrate status`
- Migrations are run in timestamp order; never edit a migration that's already been run in any environment — write a new one.
