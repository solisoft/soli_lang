# How the Soli ORM talks to SoliDB

Captured with Soli 2.28.2 against `solidb 2.4.0`: `.to_query` output, plus the
actual HTTP requests recorded by a logging proxy between Soli and SoliDB
(`SOLI_DB_DRIVER=0` forces HTTP; by default Soli uses SoliDB's binary
MessagePack driver for CRUD and queries, which carries the same SDBQL).

## Connection

Env vars (`.env`): `SOLIDB_HOST` (`http://localhost:6745`), `SOLIDB_DATABASE`,
`SOLIDB_USERNAME`, `SOLIDB_PASSWORD`. Soli logs in with
`POST /auth/login` and reuses the JWT. A bare `soli script.sl` with those vars
set and `class Post < Model end` works — handy for testing mappings.

## Naming

- Collection = snake_case plural of the class, `Model` suffix dropped:
  `Article` → `articles`, `BlogPost` → `blog_posts`, `Person` → `people`,
  `Category` → `categories`.
- The loop variable is always **`doc`**: a string `where` must say
  `doc.field`.
- A loaded record's key is `record._key` (and `record._id` =
  `"db:coll/key"`). `record.id` is `nil` on records read back by
  `all`/`first`/`find`/`find_by`; it is set only on the instance `create`
  returned. Saving that instance writes an `id` field (= the key) into the
  stored document.
- `belongs_to("author")` → foreign key `author_id` holding the parent's
  `_key`; `has_many("articles")` filters `articles.author_id == parent._key`.
  Neither declares a database foreign key (they exist since 2.10.0, see
  `http-api.md`): the reference is checked by nobody unless one is declared
  over HTTP.

## Reads

| Soli | SDBQL sent (bind vars) |
|------|------------------------|
| `Article.all` | `FOR doc IN articles RETURN doc` |
| `Article.count` | `RETURN COLLECTION_COUNT("articles")` |
| `Article.where({...}).count` | `RETURN LENGTH(FOR doc IN articles FILTER … RETURN 1)` |
| `Article.order("views").first` | `FOR doc IN articles SORT doc.views ASC LIMIT 1 RETURN doc` |
| `Article.order("views").first(2)` | same with `LIMIT 2`, returns an array |
| `Article.find(key)` | `GET /_api/database/{db}/document/articles/{key}` (no SDBQL); missing → raises `RecordNotFound` |
| `Article.find_by("title", "One")` | `FOR doc IN articles FILTER doc.title == @val LIMIT 1 RETURN doc` → `nil` on miss |
| `Article.first_by("title", "One")` | `… FILTER doc.title == @val SORT doc._key ASC LIMIT 1 RETURN doc` |
| `Article.order("views", "desc").limit(2).offset(1)` | `FOR doc IN articles SORT doc.views DESC LIMIT 1, 2 RETURN doc` |
| `Article.select("title", "views")` | `RETURN {title: doc.title, views: doc.views, _key: doc._key}` |
| `Article.pluck("title")` | `RETURN doc.title` — returns a QueryBuilder; finish with `.all` |
| `Article.where({...}).exists` | `… LIMIT 1 RETURN true` — a QueryBuilder; `.exists.first` → `true`/`false` |
| `Article.paginate({"page": 1, "per": 2})` | `RETURN COLLECTION_COUNT("articles")` then `FOR doc IN articles LIMIT 0, 2 RETURN doc` |

### `where` hash → FILTER

Each value gets a bind var named `<field>__<op>_<n>`; conditions are ANDed.

| Soli hash | SDBQL |
|-----------|-------|
| `{"published": true, "views": {"gte": 5, "lt": 100}}` | `doc.published == @published__eq_1 AND doc.views >= @views__gte_2 AND doc.views < @views__lt_3` |
| `{"title": ["One", "Two"]}` (or `{"in": […]}`) | `doc.title IN @title__in_1` (an empty list matches nothing) |
| `{"title": {"like": "T%"}}` | `LIKE(doc.title, @title__like_1, false)` — case-sensitive |
| `{"title": {"ilike": "t%"}}` | `LIKE(doc.title, @title__ilike_1, true)` — case-insensitive |
| `{"or": [{"views": 5}, {"views": 20}]}` | `(doc.views == @views__eq_1 OR doc.views == @views__eq_2)` |
| `{"nope": nil}` | `doc.nope == @nope__eq_1` (null) — also matches docs **without** the field |
| `{"title": {"ne": nil}}` | `doc.title != @title__ne_1` |
| `.where({...}).where("doc.views > @v", {"v": 6})` | `FILTER (doc.published == @published__eq_1) AND (doc.views > @v)` |

The string form is spliced verbatim into `FILTER` — developer-trusted only;
bind every external value.

### Relations

| Soli | SDBQL |
|------|-------|
| `article.author` (belongs_to) | `FOR doc IN authors FILTER doc._key == @fk LIMIT 1 RETURN doc` |
| `author.articles.all` (has_many) | `FOR doc IN articles FILTER doc.author_id == @author_id__eq_1 RETURN doc` |
| `author.articles.count` | `RETURN LENGTH(FOR doc IN articles FILTER doc.author_id == @author_id__eq_1 RETURN 1)` |
| `Author.includes("articles")` | `FOR doc IN authors LET _rel_articles = (FOR rel IN articles FILTER rel.author_id == doc._key RETURN rel) RETURN MERGE(doc, {articles: _rel_articles})` |
| `Author.includes("articles", "published == @p", {"p": true})` | adds `AND rel.published == @p` (the filter is auto-prefixed with `rel.`) |
| `Author.includes_count("articles")` | `LET _rel_articles_count = LENGTH(FOR rel IN articles FILTER rel.author_id == doc._key RETURN 1) RETURN MERGE(doc, {articles_count: _rel_articles_count})` |
| `Author.join("articles")` | `FILTER LENGTH(FOR rel IN articles FILTER rel.author_id == doc._key LIMIT 1 RETURN 1) > 0` |

### Aggregates

| Soli | SDBQL / result |
|------|----------------|
| `Sale.aggregate({"total": ["sum", "amount"]}).first` | `{total => 45}` ✓ |
| `Sale.where(…).group_by(["country"]).aggregate({"total": ["sum","amount"], "n": ["count"]}).having("total > @m", {"m": 1}).all` | `FOR doc IN sales FILTER … COLLECT country = doc.country AGGREGATE total = SUM(doc.amount), n = COUNT() FILTER total > @m RETURN {country: country, total: total, n: n}` ✓ |
| `Sale.count_distinct("country").first` | `COLLECT AGGREGATE __soli_agg = COUNT_DISTINCT(doc.country) RETURN __soli_agg` → `2` ✓ |
| `Sale.median("amount").first` | `COLLECT AGGREGATE __soli_vals = COLLECT_LIST(doc.amount) RETURN MEDIAN(__soli_vals)` → `10` ✓ |
| `Article.where(…).sum("views")` (`.first` / `.all`) | `… RETURN SUM(doc.views)` (one SUM per row) → **`nil`**. Use `.aggregate({"total": ["sum", "views"]}).first` |

`.sum`, `.avg`, `.min`, `.max` return a QueryBuilder whose `.first`/`.all`
is `nil` (Soli's models.md says so; Soli's solidb-reference.md does not).

## Writes

| Soli | HTTP / SDBQL |
|------|--------------|
| `Article.create({...})` | `POST /document/articles` with the hash |
| `record.save` (existing) | `PUT /document/articles/{key}` with all fields (merge) |
| `record.update({...})` | same as save after assigning |
| `Article.update(key, {...})` | `PUT /document/articles/{key}` with just the hash |
| `record.increment("views")` | `GET` the doc, then `PUT {"views": n+1}` |
| `record.delete` / `Article.delete(key)` | `DELETE /document/articles/{key}` |
| `Article.upsert(key, {...})` | `PUT /document/articles/{key}`; on 404 `POST` with `_key` (also creates the collection if needed) |
| `Article.find_or_create_by("title", "Foc", {...})` | `find_by` query, then `POST` — two requests, not atomic |
| `Article.where(…).update_all({"x": 1})` | `FOR doc IN articles FILTER … UPDATE doc WITH @__soli_update IN articles` |
| `Article.where(…).delete_all` | `FOR doc IN articles FILTER … REMOVE doc IN articles` |

Because `UPDATE`/`PUT` merges only top-level fields, a nested-object field in
`update_all` replaces the stored object (SoliDB default `mergeObjects: false`).

## `grouped`, transactions

```soli
grouped(fn() {
  count = Article.count
  two = Article.find_by("title", "Two")
  everyone = Author.all
})
```

sent **one** cursor request:

```sdbql
LET _b0 = (COLLECTION_COUNT("articles"))
LET _b1 = (FOR doc IN articles FILTER doc.title == @b1__val LIMIT 1 RETURN doc)
LET _b2 = (FOR doc IN authors RETURN doc)
RETURN [_b0, _b1, _b2]
```

(Assign to variables that exist outside the block, or `@ivars` in a
controller. Under `soli serve --dev` the reads are not coalesced — per Soli
docs, unverified here.)

| Soli | HTTP |
|------|------|
| `Model.transaction do … end` | `POST /transaction/begin {"database":…,"isolationLevel":"read_committed"}`, each write `POST /transaction/tx:…/document/{coll}`, then `/commit` — or `/rollback` if the block raises (verified: row not persisted, exception re-raised). Queries in the block (`where`, `update_all`, `delete_all`, `@sdbql{}`) go to `POST /transaction/tx:…/query` with `bind_vars` after Soli 2.34; **in 2.34 and earlier they went to `/cursor` or the native driver, outside the transaction: an `update_all` or an `@sdbql` write applied at once and survived the rollback** (verified, Soli 2.34.0 against SoliDB 2.9.0) |
| `Model.transaction("FOR … REMOVE …")` | a plain `POST /cursor` with the statement (a single statement is atomic anyway) |

## Raw SDBQL from Soli

```soli
min_views = 6
titles = @sdbql{ FOR a IN articles FILTER a.views >= #{min_views} SORT a.views RETURN a.title }
# ["One", "Three"] — #{} becomes a bind var; rows are plain hashes, not model instances

db = Solidb("http://localhost:6745", "mydb")
db.auth("admin", "secret")
db.query("FOR d IN articles FILTER d.views > @v RETURN d.title", {"v": 6})   # ["One", "Three"]
```

Failure behavior (all verified):

| Call with an invalid query | Result |
|----------------------------|--------|
| `Model.where("doc.views >>> 1").all` | raises |
| `….count` | raises |
| `….first` | **`nil`** (indistinguishable from "no row") |
| `@sdbql{ FOR a IN articles FILTR a RETURN a }` | returns the **String** `Error: Query failed: 400 Bad Request - {"error":"Parse error: …"}` |
| `db.query("…bad…")` | raises |
| `Model.where("NOPE_FN(doc.title)").all` | `[]` (unknown function inside FILTER matches nothing) |
| `db.query("FOR d IN @@coll …", {"@coll": "articles"})` | raises — SoliDB has no `@@` binds |
| `db.get("articles", "missing")` | raises (use `rescue nil`) |

Check an `@sdbql` result with `type(result) == "string"` before using it.

## Search helpers

| Soli | Sent |
|------|------|
| `Person.search("coffee", {"limit": 2})` (needs a fulltext index) | `LET __soli_hits = FULLTEXT("people", "bio", @__soli_q, 2) FOR r IN __soli_hits LIMIT 2 RETURN MERGE(r.doc, {_search_score: r.score})` |
| `Place.near(48.86, 2.33, {"limit": 1})` | `POST /geo/places/loc/near {"lat","lon","limit"}` |
| `Place.within(48.86, 2.33, 1000)` | `POST /geo/places/loc/within {"lat","lon","radius"}` |
| `Doc.similar([1,0,0], "emb", 2)` | `POST /vector/docs/idx_emb/search {"limit":2,"vector":[…]}` — index must be named `idx_<field>` |
