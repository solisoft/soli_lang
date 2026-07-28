# Django benchmark app

The Django column of `www/docs/benchmarks.md`, serving the same matched
workloads as the Soli, Rails, Express and Laravel apps.

## Shape

* **gunicorn with 16 workers** on port **5099**, matching every other stack.
* **`DEBUG = False`**, so templates use Django's cached loader.
* **Django ORM + Django templates** — the ORM and template engine a Django app
  actually uses.
* **Persistent connections** (`CONN_MAX_AGE = None`). Without them every request
  opens a fresh PostgreSQL connection, worth ~8ms on loopback; that is the
  analogue of the pool the other stacks hold, and skipping it measures
  connection setup rather than Django.
* **Compact JSON separators.** `json.dumps` defaults to `', '` / `': '`, which
  would make Django's payload 299 bytes larger than every other stack's for the
  same 50 rows. With them the JSON and DB responses are byte-identical at 2,268
  bytes.

The `posts` (50 rows) and `wposts` (800,000 rows) tables are created and seeded
by the shared harness, so both models are `managed = False`.

## Running

```bash
pip install --user django gunicorn 'psycopg[binary]'
python3 -m gunicorn --workers 16 --bind 127.0.0.1:5099 \
  --access-logfile /dev/null benchproj.wsgi:application
```

## Endpoints

| Route | Workload |
|---|---|
| `GET /json` | 50 in-memory objects as JSON |
| `GET /template` | the same 50 rows through a Django template |
| `GET /db` | 50 rows projected in the database, as JSON |
| `GET /db-template` | the same read, rendered as HTML |
| `GET /db-hydrated` | reference: the form that instantiates 50 model objects |
| `POST/PATCH/DELETE /w` | one create / update / delete per request |
