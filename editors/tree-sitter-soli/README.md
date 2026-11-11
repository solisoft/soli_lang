# tree-sitter-soli

A [tree-sitter](https://tree-sitter.github.io/) grammar for Soli (`.sl`), for
the editors that highlight, fold and indent with tree-sitter: Neovim, Helix,
Zed, Emacs 29+. `.slv` views need no grammar of their own: they use
`embedded_template` (the one ERB and EJS use) with the queries in
`template-queries/`, which send the markup to HTML and the code of every tag
to Soli.

The tree is for editors, not for running code. Where Soli's parser decides
from context, the grammar keeps both readings and lets the one that parses
win (`match kind {` opens the match; `if xs.any? { |x| … }` passes a block).

## What it covers

Parsed without an error, in this repository (CI checks the first two):

| Corpus | Files | Without an error |
|---|---|---|
| `.sl` under `tests/`, `examples/`, `www/`, `src/scaffold/` | 373 | 359 (96%) |
| `.slv` views, tags read as one program | 230 | 230 (100%) |

The 14 `.sl` files left are language specs that exercise rare forms: a hash
comprehension with a computed key (`{u["name"]: u["age"] for u in users}`),
a parenthesis-less call on a receiver (`user.update name: "Bob"`), a bare
`{ … }` scope block, `class << self`, `count!=1` with no spaces, and a
lambda called on the spot (`|x| { … }()`).

## Layout

- `grammar.js` — the grammar. `src/parser.c` and `src/grammar.json` are
  generated from it (`tree-sitter generate`); commit them together.
- `src/scanner.c` — line breaks, string contents, `"""` strings,
  `@sdbql{ … }` blocks and nested `/* */` comments. Soli ends a statement at a
  line break, except inside brackets, after an operator, and before a line
  that starts with `.`, `&.`, `|>`, `&&`, `||`, `??` or a ternary's `?`/`:`.
  In a template, each tag starts a statement.
- `queries/` — `highlights`, `locals`, `folds`, `indents`, `injections`.
- `template-queries/` — `highlights` and `injections` for `.slv` files.
- `test/corpus/` — `tree-sitter test`.

## Working on it

```bash
cd editors/tree-sitter-soli
tree-sitter generate     # after editing grammar.js
tree-sitter test         # the corpus in test/corpus
tree-sitter parse ../../tests/language/classes_spec.sl
```

The CI job (`.github/workflows/tree-sitter.yml`) regenerates the parser and
fails when the committed `src/` differs, runs `tree-sitter test`, and parses
the repository's `.sl` files, failing below 95% without an error.

## Editors

The parser is built from this directory of the Soli repository: each editor
below takes the repository, a revision and the subdirectory
`editors/tree-sitter-soli`. `soli lsp` (diagnostics, completion, go to
definition…) is configured separately; see
[Editor integration](https://soli.solisoft.net/docs/development-tools/editor-integration).

### Neovim 0.11+

Without a plugin: build the parser and copy the queries into your runtime
path.

```bash
git clone --depth 1 https://github.com/solisoft/soli_lang /tmp/soli
cd /tmp/soli/editors/tree-sitter-soli
mkdir -p ~/.local/share/nvim/site/parser ~/.local/share/nvim/site/queries/soli
tree-sitter build -o ~/.local/share/nvim/site/parser/soli.so
cp queries/*.scm ~/.local/share/nvim/site/queries/soli/
```

```lua
vim.filetype.add({ extension = { sl = "soli", slv = "soli_template" } })
vim.api.nvim_create_autocmd("FileType", {
  pattern = "soli",
  callback = function() vim.treesitter.start() end,
})
```

For `.slv`, register the `embedded_template` parser (from nvim-treesitter:
`:TSInstall embedded_template html`) under its own name, so it reads the
queries in `template-queries/` instead of ERB's, which inject Ruby:

```bash
mkdir -p ~/.local/share/nvim/site/queries/soli_template
cp template-queries/*.scm ~/.local/share/nvim/site/queries/soli_template/
```

```lua
local template = vim.api.nvim_get_runtime_file("parser/embedded_template.so", false)[1]
if template then
  vim.treesitter.language.add("soli_template", { path = template, symbol_name = "embedded_template" })
  vim.api.nvim_create_autocmd("FileType", {
    pattern = "soli_template",
    callback = function() vim.treesitter.start() end,
  })
end
```

### Helix

In `~/.config/helix/languages.toml`:

```toml
[[language]]
name = "soli"
scope = "source.soli"
file-types = ["sl", { glob = "Solifile" }]
roots = ["soli.toml", "config/routes.sl"]
comment-token = "#"
indent = { tab-width = 2, unit = "  " }
language-servers = ["soli"]

[[grammar]]
name = "soli"
source = { git = "https://github.com/solisoft/soli_lang", rev = "main", subpath = "editors/tree-sitter-soli" }

[[language]]
name = "soli-template"
scope = "text.html.soli"
file-types = ["slv"]
roots = ["soli.toml", "config/routes.sl"]
grammar = "embedded-template"
injection-regex = "soli-template"
language-servers = ["soli"]
```

Then `hx --grammar fetch && hx --grammar build`, and copy the queries:
`queries/*.scm` into `~/.config/helix/runtime/queries/soli/`, and
`template-queries/*.scm` into `~/.config/helix/runtime/queries/soli-template/`.
Pin `rev` to a commit for a reproducible build.

### Zed

Zed takes a grammar from an extension. In a dev extension
(`zed: install dev extension`), `extension.toml` declares:

```toml
[grammars.soli]
repository = "https://github.com/solisoft/soli_lang"
rev = "<commit>"
path = "editors/tree-sitter-soli"
```

with `languages/soli/config.toml` (`name = "Soli"`, `grammar = "soli"`,
`path_suffixes = ["sl"]`, `line_comments = ["# "]`) and the files of
`queries/` copied next to it.

## License

MIT, like Soli.
