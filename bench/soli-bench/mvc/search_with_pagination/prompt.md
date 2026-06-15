# search_with_pagination

Build a controller action that searches `Article` records by title and returns
a paginated, ordered result.

Implement, in `stub.sl`:

- `class Article extends Model`
- `search_articles(params)` where `params` may contain:
  - `"q"`    — a case-insensitive substring to match against the title
               (default: `""`, which matches everything)
  - `"page"` — 1-based page number (default: `1`)

  Use a page size of **2**. Return:

  ```soli
  {
    "status": 200,
    "titles": [<matching titles on this page, ordered A→Z>],
    "total": <total number of matches>,
    "total_pages": <number of pages>
  }
  ```

Idiomatic touches:

- Use `Model.where(...)` with a bind variable, `.order(...)`, and
  `.paginate({"page": ..., "per": ...})`.
- `paginate` returns `{"records": [...], "pagination": {...}}` where the
  pagination hash has `page`, `per`, `total`, and `total_pages`.
- Use `||` for the parameter defaults.

## Requires SoliDB

Export `SOLIDB_HOST`, `SOLIDB_USERNAME`, and `SOLIDB_PASSWORD` before grading
(see the suite README).
