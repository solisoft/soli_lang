# Kemal (Crystal)

Kemal 1.14 + Jennifer 0.13 (ORM) + ECR (templates) + crystal-pg, on Crystal 1.21.1.
Port **5105**.

Kemal is a Sinatra-style micro-framework: it ships neither an ORM nor a view layer
beyond Crystal's compile-time ECR. Jennifer is added the way Sequelize is added to
Express, and the column is named after both.

* **16 processes, one thread each.** Crystal's multi-threading is still behind
  `-Dpreview_mt`, so `start-bench.sh` starts 16 copies of the binary on the same
  port with `SO_REUSEPORT` — the shape of the forking stacks, not of Soli's or
  Phoenix's single process.
* **80 connections**: a Jennifer pool of 5 per process.
* **Reads project without instantiating models**: `Post.all.pluck(:id, :title, :views)`,
  serialised as `{id, title, views}` objects — byte-identical `/json` and `/db`
  (2,268 bytes) and the same 2,864-byte page as Laravel, Django and FastAPI.
* **Writes**: `Wpost.create` for the insert; update and delete are query-level
  (`Wpost.where { _id == key }.update(...)` / `.delete`), like Django's
  `filter(...).update()`. Keys are drawn from `1..WPOOL`.

**Opt-in, not published**: Kemal is not comparable feature for feature (no sessions,
CSRF or security headers; the ORM and escaping were added), so it is not in the default
`STACKS` and `start.sh` does not start it. To measure it:

```bash
cd kemal && shards install && shards build --release --production
setsid nohup ./start-bench.sh > /tmp/bench-kemal.log 2>&1 < /dev/null &   # under SERVER_CPUS
cd .. && STACKS=kemal ./sweep.sh
```

`mise.toml` at the suite root pins Crystal 1.21.1.
