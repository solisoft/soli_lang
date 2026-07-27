# Laravel benchmark app

The Laravel column of `www/docs/benchmarks.md`, serving the same matched
workloads as the Soli, Rails and Express apps.

Laravel runs in Docker because this box has no `php-fpm` and no `pgsql` PHP
extension; the container supplies both. It uses **host networking**, so it pays
no NAT overhead the native stacks avoid and reaches the same PostgreSQL on
`127.0.0.1:5433` that Rails and Express use.

## Shape

* **php-fpm 8.4, `pm = static`, `pm.max_children = 16`** — 16 workers, matching
  every other stack.
* **nginx** on port **5098**.
* **OPcache** with `validate_timestamps=0`, and `config:cache` / `route:cache` /
  `view:cache` — production settings, so the framework is not measured
  unoptimised.
* **Eloquent + Blade**, the ORM and template engine a Laravel app actually uses.
* **Persistent PDO connections.** php-fpm holds no connection pool, so without
  them every request pays a fresh PostgreSQL connect — worth ~8ms on loopback,
  and it cost this app two thirds of its database throughput (1,498 -> 4,286
  req/s). They are the php-fpm equivalent of the pool the other three hold.

This is the ordinary php-fpm deployment, **not** Laravel Octane. Octane keeps
the application resident between requests and is substantially faster; measuring
it and calling the result "Laravel" would flatter it the same way the raw `pg`
driver once flattered Express. If Octane is added it belongs as a separate,
labelled row.

## Running

```bash
docker compose build
docker compose up -d
curl -s localhost:5098/db | head -c 80
```

The app expects the `posts` (50 rows) and `wposts` (800,000 rows) tables the
shared harness seeds. First-time setup needs the vendor tree:

```bash
docker run --rm -v "$PWD/app":/app -w /app --user "$(id -u):$(id -g)" \
  -e COMPOSER_HOME=/tmp/composer composer:latest install --no-interaction
cp app/.env.example app/.env   # then set DB_* to the bench database
```

## Endpoints

| Route | Workload |
|---|---|
| `GET /json` | 50 in-memory objects as JSON |
| `GET /template` | the same 50 rows through Blade |
| `GET /db` | 50 rows projected in the database, as JSON |
| `GET /db-template` | the same read, rendered as HTML |
| `GET /db-hydrated` | reference: the canonical Eloquent form that instantiates 50 models |
| `POST/PATCH/DELETE /w` | one create / update / delete per request |
