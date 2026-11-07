# Editor Integration

Soli ships a language server, `soli lsp`, spoken over stdio. The VS Code /
Cursor and Nova extensions start it, Neovim and Helix take a few lines of
configuration, and any editor with a generic LSP client can use it. The
server reports its version in `initialize` (`serverInfo`).

## What the language server does

| Feature | What you get |
|---|---|
| Diagnostics | The lint rules as warnings while you type, and a syntax error as an error at its position. A file under `app/` gets the path-aware rules too (`security/unfiltered-mass-assignment`, `style/redundant-model-import`…), with the names its auto-loaded neighbours declare in scope, as `soli lint` does. A `.slv` template has its `<% %>` code linted, not its HTML: an unclosed tag or an `<% if %>` without its `end` is an error on its line, and a finding covers its whole line. |
| Hover | On a name declared in the file — at its declaration or at any use — its kind or declared type (`greet : function`). On a name declared elsewhere in the application — `Post` in a controller, a helper in a view, `post.summary` — its signature and the file and line it is in. On a common builtin (`print`, `len`, `json_parse`, `HTTP`, `DateTime`…), its signature. |
| Completion | Keywords, types, the names declared in the file, and the application's classes and functions. After `.` or `@`, the receiver's members — see below. |
| Go to definition | The declaration of the name under the cursor: in the file first, then anywhere in the application's auto-loaded files (`app/models`, `app/controllers`, `app/services`, `app/policies`, `app/mailers`, `app/middleware`, `app/jobs`, `app/subscribers`, and `app/helpers` from a helper or a view). `Post` in a controller lands on `class Post` in `app/models/post.sl`; `post.summary` on the `def summary` of the class `post` holds. A name declared twice answers with both. |
| References | Every use of the name in the file, `#{…}` interpolations included — not words in plain strings or comments. |
| Rename | The same occurrences, in one edit. A new name that is not an identifier is refused. |
| Document symbols | An outline of the file's classes, functions and variables. |
| Folding | Classes and functions, and multi-line `{ }`, `( )` and `[ ]` (hashes, arrays, long calls). |
| Formatting | The whole document, with the same formatter as `soli fmt`. A file that does not parse is left alone. |
| Code actions | On a lint finding: rename to the expected case for the naming rules (every occurrence), or insert `# soli-lint-disable-next-line <rule>` above the line. |

Formatting, document symbols, folding, code actions, references and rename
are for `.sl` files; on a `.slv` template they answer nothing (rename would
otherwise reach words in the markup). In a template, completion works inside
`<% %>` tags only, and nowhere does a `.` or `@` in a comment or a string open
the member list. A file that is neither `.sl` nor `.slv` — an HTML buffer an
editor sends along — gets nothing from the server.

### Member completion

Typing `.` or `@` asks for the members of what is before it. The server reads
that receiver from the text, without parsing — while you type `post.` the file
usually does not parse — and offers members only when it can tell exactly
what the receiver is:

| Receiver | Members offered |
|---|---|
| A class: `Post.` | Its `static def`s and its `scope("…")`s, inherited ones included, `new`, and — for a model — `Model`'s class methods (`where`, `find_by`, `create`…). |
| An instance: `post.` after `post = Post.find(id)` (or `Post.new(…)`, `find_by`, `first_by`, `first`, `last`, `create`), or a parameter typed `post: Post` | The class's methods, fields and `has_many` / `has_one` / `belongs_to` relations, inherited ones included, and — for a model — `Model`'s instance methods (`save`, `update`, `errors`…). |
| `@` or `this.` inside a class | The same, for the enclosing class. |
| A literal, or a local assigned one: `"x".`, `[1, 2].`, `{…}.`, `42.` | The built-in methods of a string, an array, a hash, a number. |
| Anything else (`a.b.`, a local of unknown origin) | Every method name declared in the application, marked *receiver not resolved*. |

The application is indexed once, on the first request that needs it, then
kept current file by file: an open buffer on every change (a buffer that does
not parse keeps its last good version), a file on save, and — when the editor
supports watching files — a file changed, added or deleted outside the editor.

**Limits.** References and rename still work within one file, and names are
matched by spelling there, so two variables called `count` in two functions
count as one name. Member completion follows one step: `post.` is resolved,
`post.author.` is not. Instance variables a controller sets are not offered in
its views.

## Requirements

- The `soli` binary on your `PATH` (`soli --version` should work), or its
  absolute path in the editor's settings.
- Files with the `.sl` extension, and `.slv` view templates — the editor must
  send those to the server too (see the Neovim and Helix setups below).
- A `soli.toml` at the project root, so editors find the workspace.

## VS Code & Cursor

The extension in `editors/vscode/` (`solilang.soli-language`) starts
`soli lsp`, so VS Code, Cursor, VSCodium and Windsurf get everything in the
table above, and highlights `.sl` files and `.slv` templates. It needs VS Code
1.82 or a build based on it.

It is not on the Marketplace or Open VSX yet. Download `soli-language.vsix`
from the artifacts of the **vscode extension** workflow run on GitHub, or
build it:

```bash
cd editors/vscode
npm ci
npm run package        # writes soli-language.vsix
code --install-extension soli-language.vsix
# Cursor:
cursor --install-extension soli-language.vsix
```

A 0.2 install upgrades in place: the extension id is unchanged.

### Languages

| Language | Files | What the server does |
|---|---|---|
| **Soli** (`soli`) | `.sl`, `Solifile` | Everything above. |
| **Soli Template** (`soli-template`) | `.slv` | Diagnostics, hover, go-to-definition, and completion inside `<% %>`. Highlighting: HTML with Soli inside `<% %>`, `<%= %>`, `<%- -%>`, and `<%# %>` comments. |

### Finding `soli`

The extension runs the first `soli` it finds:

1. the `soli.lsp.path` setting (an absolute path, `~/…`, or a name looked up
   on `PATH`);
2. `soli` on `PATH`;
3. `~/.local/bin/soli` (where `install.sh` puts it), `~/.cargo/bin/soli`
   (`$CARGO_HOME/bin`), `/usr/local/bin/soli`, and `/opt/homebrew/bin/soli`
   on macOS. On Windows: `soli.exe` on `PATH`, then
   `%USERPROFILE%\.cargo\bin` and `%USERPROFILE%\.local\bin`.

An editor started from the dock or the start menu often lacks your shell's
`PATH`, which is why the install directories are tried. When nothing is
found, a notification lists every place it looked, with a button to set the
path. A `soli.lsp.path` that does not exist is reported, not replaced by
another `soli`.

### Settings

| Setting | Default | |
|---|---|---|
| `soli.lsp.enabled` | `true` | Run the language server. |
| `soli.lsp.path` | `""` | Absolute path to `soli`. Empty: the search above. |
| `soli.lsp.trace` | `"off"` | `messages` or `verbose` traces the LSP traffic in the **Soli** output. |
| `soli.lint.enable` | `true` | Run `soli lint` when the language server is not running (see below). |
| `soli.lint.onSave` | `true` | Lint again on every save (fallback only). |
| `soli.lint.executablePath` | `""` | Deprecated: read only when `soli.lsp.path` is empty. |

The `soli.lsp.*` names are the Nova extension's. `soli.lsp.path` and
`soli.lint.executablePath` are read from your user settings only: a
repository's `.vscode/settings.json` cannot make the editor run another
binary. The extension stays off in a workspace you have not trusted.

### When the server cannot run

`soli lint <file>` on open and save takes over only when the server is not
running: `soli.lsp.enabled` is off, the `soli` found was built without the
`lsp` cargo feature (the extension recognises its refusal and does not
restart it), or the server stopped five times in three minutes. While the
server runs the fallback is off, so each finding is reported once — the
server applies the same rules, path-aware ones included.

### Commands

- **Soli: Restart Language Server** — picks up a rebuilt `soli`.
- **Soli: Show Output** — the server's log and trace.
- **Soli: Lint Current File** — the fallback lint, on demand.

### Grammars

The TextMate grammars live in `editors/grammars/` (`soli.tmLanguage.json`,
scope `source.soli`; `soli-template.tmLanguage.json`, scope
`text.html.soli`). `scripts/sync-grammars.sh` copies them into the VS Code
and Nova bundles, and CI fails when a copy is stale (`--check`). A Rust test
fails when the grammar's keywords and the lexer's disagree. Any editor that
reads TextMate grammars (Sublime Text, TextMate, JetBrains via its TextMate
bundle support) can load these files.

## Nova (macOS)

The Nova extension lives under `editors/nova/soli.novaextension/` and uses
Nova's `LanguageClient` API to spawn `soli lsp` on stdio.

### Install from the bundled source

1. Build and install the `soli` binary so the LSP backend is available:
   ```bash
   cargo install --path . --locked
   # or download a release binary from
   # https://github.com/solisoft/soli_lang/releases
   ```
2. Symlink the extension bundle into Nova's extensions directory:
   ```bash
   ln -s "$PWD/editors/nova/soli.novaextension" \
     "$HOME/Library/Application Support/Nova/Extensions/com.solilang.soli.novaextension"
   ```
3. Launch (or restart) Nova. Open any `.sl` file — diagnostics, hover, and
   completion should activate within a second. `.slv` templates are sent to
   the server as the **Soli Template** language, for their diagnostics.

### Install from the Extension Library

Once the extension is published, just open Nova → **Extensions** → search
**Soli** → **Install**.

### Settings (Nova → Preferences → Extensions → Soli)

| Setting | Description |
|---|---|
| `soli.lsp.enabled` | Toggle the language server. Off = grammar-only highlighting. |
| `soli.lsp.path` | Absolute path to `soli`. Leave blank to use `PATH`. |
| `soli.lsp.trace` | LSP trace verbosity (`off` / `messages` / `verbose`). |

Nova doesn't inherit your shell's full `PATH` by default. If
`soli.lsp.enabled` is on but you see no diagnostics, set
**Soli > Soli binary** to the absolute path of your `soli` executable (for
example, `/Users/you/.cargo/bin/soli`).

### Commands

- **Editor → Extensions → Restart Soli Language Server** — picks up a freshly
  rebuilt `soli` binary without reloading Nova.

## Neovim

Neovim 0.11+ configures the server natively. Two things are needed besides
the server itself: `.sl` is S-Lang (`slang`) to Neovim by default, so map it
to a `soli` filetype first — and `.slv` to a filetype of its own, so templates
reach the server for their diagnostics — and `nvim-lspconfig` has no `soli`
entry, so declare the server yourself:

```lua
-- ~/.config/nvim/after/plugin/soli.lua (or anywhere in your config)
vim.filetype.add({ extension = { sl = "soli", slv = "soli_template" } })
-- Highlight templates as HTML with embedded code.
vim.treesitter.language.register("embedded_template", "soli_template")

vim.lsp.config("soli", {
  cmd = { "soli", "lsp" },
  filetypes = { "soli", "soli_template" },
  root_markers = { "soli.toml", "config/routes.sl", ".git" },
})

vim.lsp.enable("soli")
```

Open a `.sl` file and `:checkhealth vim.lsp` lists the `soli` client. With
Neovim's default mappings, `K` hovers, `CTRL-]` jumps to the definition, `grr`
lists references, `grn` renames and `gra` offers the code actions; format with
`:lua vim.lsp.buf.format()`.

## Helix

In `~/.config/helix/languages.toml`:

```toml
[language-server.soli]
command = "soli"
args = ["lsp"]

[[language]]
name = "soli"
scope = "source.soli"
file-types = ["sl"]
roots = ["soli.toml", "config/routes.sl"]
comment-token = "#"
language-servers = ["soli"]

[[language]]
name = "soli-template"
scope = "text.html.soli"
file-types = ["slv"]
roots = ["soli.toml", "config/routes.sl"]
grammar = "embedded-template"
language-servers = ["soli"]
```

## Other editors

Any editor with an LSP client can start `soli lsp` on stdio for `.sl` and
`.slv` files. The shape of the entry, in the style most clients use:

```json
{
  "name": "soli",
  "command": ["soli", "lsp"],
  "filetypes": ["soli", "soli_template"],
  "fileExtensions": [".sl", ".slv"],
  "rootPatterns": ["soli.toml", "config/routes.sl"],
  "languageId": "soli"
}
```

The server tells a template by its `.slv` extension, whatever language id the
client sends. It reads the application root from the workspace folders the
client sends in `initialize`, or from the nearest `soli.toml` or
`config/routes.sl` above the open file.
