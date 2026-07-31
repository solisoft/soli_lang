# Static & Markdown Server

`soli serve` works on any folder, not just Soli apps.

Point it at a directory that has no controllers and no routes file and it serves that directory
as a website: files straight off disk, `.md` rendered as styled HTML pages, `.slv`/`.erb`
templates executed, and a generated index for every folder and sub-folder with a sidebar
carrying the whole tree.

```bash
soli serve ./notes --dev
# Serving files from /home/you/notes
# Server listening on http://127.0.0.1:5011
```

No config file, no scaffolding, no build step. It is the same binary you already have.

## How the mode is chosen

`soli serve <folder>` looks for two markers:

| Marker | Result |
|--------|--------|
| `app/controllers/` exists | MVC app |
| `config/routes.sl` exists | MVC app |
| neither | file mode |

A `.soli` bundle is always an app.

Override the detection when you need to:

```bash
soli serve .              # detect
soli serve . --static     # force file mode, even inside a Soli app
soli serve . --app        # require an app; fail if the folder is not one
```

`--app` keeps the original behaviour, including its error:

```
Error: Invalid MVC structure: /path/app/controllers does not exist. Expected app/controllers/ directory.
```

`--static` is useful for reading a project's own `docs/` folder, or for checking what a build
output directory actually contains.

## What gets served

A request is resolved in this order:

| # | Request | Response |
|---|---------|----------|
| 1 | any path segment starting with `.` | `404` — dotfiles are invisible |
| 2 | a path escaping the folder | `403` |
| 3 | a folder containing an index document | that document, by its own rule |
| 4 | a folder | generated index, with `README.md` rendered above it |
| 5 | `*.md`, `*.markdown` | rendered Markdown page |
| 6 | `*.html.slv`, `*.slv`, `*.html.erb`, `*.erb` | template executed by the engine |
| 7 | `/about` (no extension) | first of `about.md`, `.html`, `.html.slv`, `.slv`, `.html.erb`, `.erb` |
| 8 | any other file | served as-is, with MIME type, `ETag` and `Range` support |
| 9 | nothing matches | `404` page naming the nearest folder |

`GET` and `HEAD` are answered; anything else gets a `405`.

Requesting a folder without a trailing slash redirects to the slash-terminated URL (`301`), so
relative links inside a `README.md` resolve against the right base.

### Index documents

`index.*` means "this **is** the page for this folder", so it replaces the generated listing
entirely. Looked up in this order, and each one is then served by its own rule:

| File | Result |
|------|--------|
| `index.html`, `index.htm` | served as-is |
| `index.md` | rendered as a Markdown page |
| `index.html.slv`, `index.slv` | executed by the template engine |
| `index.html.erb`, `index.erb` | executed by the template engine |

A `README.md` is the opposite: it *describes* a folder rather than replacing it, so it renders
above the listing. Same split as GitHub and every static host.

A folder can hold both, but an index wins outright: with `index.md` **and** `README.md`, only the
index renders — the README is still reachable at its own URL, but nothing links to it any more.

### Folder pages

A folder with no index document gets a listing of its contents — folders first, then files, with
size and last edit for each. If it has a `README.md` (or `readme.md`), that renders above the
listing, so a documented directory reads as a page with its contents underneath.

Folder indexes read their directory directly and always list everything.

### Markdown pages

Rendered with tables, strikethrough and task lists enabled — the same converter and the same URL
safety policy as `.md` views inside an app: a `[link](javascript:…)` is neutralized to `#`.

Fenced `soli` and `sl` code blocks are syntax-highlighted server-side by re-lexing them with
Soli's own lexer. Other languages render as plain monospace with the fence's info string shown as
a label — no guessing, and nothing is fetched from a CDN.

The page title comes from the document's first `#` heading, falling back to the filename.

Every `##` and `###` gets a slug anchor and appears in an **On this page** rail on the right,
which marks the section you are reading as you scroll. The rail is hidden below 1180px, and a
document with fewer than two headings gets none — a one-line table of contents is furniture, not
navigation.

### Media and other files

Clicking a picture in a listing keeps you in the site: images, video, audio and PDFs open inside
the shell, with the breadcrumb, the sidebar and the file's size and type. Text and source files
are shown in the page too — `.sl` and `.slv` with the same lexer highlighting as a fenced block —
up to 512 KB, past which they stay a download. Anything the browser cannot show offers a download
link rather than dumping bytes at you.

The raw file is always still reachable, which matters: an `<img>` embedded in a Markdown page
needs the picture, not a page about the picture. The two are told apart by what the browser asks
for — a click is a navigation (`Sec-Fetch-Dest: document`), an `<img>` is a subresource. Tools
that send neither header, like `curl` and `wget`, get the bytes.

Append `?raw` to force the file for anything:

```bash
curl -O http://127.0.0.1:5011/images/logo.png?raw
```

That is what the viewer's own `<img>` tag and its download link point at, so it can never recurse
into itself.

### Templates

`.slv` and `.erb` files are executed by the template engine, with the served folder as the views
root — so `about.html.slv` is `/about` and `guides/setup.slv` is `/guides/setup`.

Templates receive two locals:

| Local | Value |
|-------|-------|
| `path` | the request path |
| `params` | query-string parameters as a hash |

```erb
<h1>About</h1>
<p>You asked for <%= h(params["name"]) %>.</p>
```

Rendered without a layout — a plain directory has no `layouts/application`, and wrapping someone's
standalone page in one they never wrote would be a surprise. Pull one in with `partial()` if you
want it. Partials, helpers and every other template feature work normally.

The generated shell is not applied either: a template's output is your HTML, so it has no sidebar,
breadcrumb or outline rail. Worth knowing for `index.slv`, which replaces a folder's page — where
`index.md` arrives wrapped in the shell, `index.slv` does not.

A template that fails to render returns a `500` page naming the file and the error.

> **Templates are code.** File mode does not load `.env`, does not open a database connection and
> does not run controllers — but a `.slv` file in the folder *is* executed. Do not point
> `soli serve` at a directory whose contents you do not control.

## The generated pages

Pages are styled in Soli's solar theme, in two states of the same sun: a night palette and a day
palette, following `prefers-color-scheme` with a toggle in the top bar that overrides it.

The sidebar is a `tree(1)` listing drawn with real box-drawing glyphs, and its vertical rail
lights up along the chain of folders leading to the page you are on.

| Key | Action |
|-----|--------|
| `/` | focus the filter |
| `↑` `↓` | move through matches |
| `Enter` | open the highlighted entry |
| `Esc` | clear the filter, then leave the field |

The sidebar scrolls to centre the file you are reading, so on a deep tree the lit rail is
actually in view. It is rendered server-side and every row is a real link, so it works with
JavaScript disabled; the script only narrows what is already there.

The stylesheet and script are compiled into the binary and served from `/__soli/files.css` and
`/__soli/files.js`. Generated pages make no network request at all — `soli serve` works offline.

Very large trees are capped at 1000 sidebar entries, and the sidebar says so when it truncates.
Folder pages are never truncated.

## Live reload

With `--dev`, editing any file in the folder refreshes the browser — the same live-reload channel
Soli apps use. Edit a `README.md`, save, and the page updates.

```bash
soli serve ./docs --dev
```

## Binding and exposure

File mode binds `127.0.0.1` by default. Serving a directory you happened to `cd` into should not
publish it to your network without you saying so.

To expose it deliberately:

```bash
SOLI_HOST=0.0.0.0 soli serve ./public-notes --port 8080
```

MVC apps still default to `0.0.0.0`, unchanged.

## Security

| Behaviour | Detail |
|-----------|--------|
| Dotfiles hidden | `.env`, `.git/`, `.ssh/` return `404`, and never appear in a listing |
| Path jail | every request is canonicalized and checked against the root; symlinks that escape return `403` |
| No `.env` loading | file mode never reads environment files from the folder |
| No database | no SoliDB connection is configured or opened |
| No controllers | no `.sl` file is executed; only `.slv`/`.erb` templates render |
| File builtins jailed | `File.*` and `Image.*` cannot read or write outside the served folder |

The dotfile rule returns `404` rather than `403` on purpose: a `403` would confirm the file
exists.

## Flags

| Flag | Meaning |
|------|---------|
| `--static` | force file mode |
| `--app` | require an MVC app |
| `--dev` | live reload |
| `--port PORT` | port (default `5011`) |
| `--workers N` | worker threads |
| `-d` | daemonize |

## Examples

```bash
# Read a project's docs folder as a site
soli serve ./docs --dev

# Browse a build output directory
soli serve ./dist

# Read your own notes, with live reload while you write
soli serve ~/notes --dev

# Read a Soli app's docs without booting the app
soli serve ./my_app/docs --static

# Share on the LAN, deliberately
SOLI_HOST=0.0.0.0 soli serve ./handbook --port 8080
```
