# Editor Integration

Soli ships a language server, `soli lsp`, spoken over stdio. Nova and Neovim
use it today, and so can any editor with a generic LSP client. The VS Code /
Cursor extension does not start it yet: it runs `soli lint` instead (see
below).

## What the language server does

| Feature | What you get |
|---|---|
| Diagnostics | The lint rules as warnings while you type, and a syntax error as an error at its position. |
| Hover | On a name declared in the file — at its declaration or at any use — its kind or declared type (`greet : function`). On a common builtin (`print`, `len`, `json_parse`, `HTTP`, `DateTime`…), its signature. |
| Completion | Keywords, types, and the names declared in the file. No member completion after `.` yet. |
| Go to definition | The declaration of the name under the cursor, in the same file. |
| References | Every use of the name in the file, `#{…}` interpolations included — not words in plain strings or comments. |
| Rename | The same occurrences, in one edit. A new name that is not an identifier is refused. |
| Document symbols | An outline of the file's classes, functions and variables. |
| Folding | Classes and functions, and multi-line `{ }`, `( )` and `[ ]` (hashes, arrays, long calls). |
| Formatting | The whole document, with the same formatter as `soli fmt`. A file that does not parse is left alone. |
| Code actions | On a lint finding: rename to the expected case for the naming rules (every occurrence), or insert `# soli-lint-disable-next-line <rule>` above the line. |

**Limits.** Everything works on one file at a time: a model used in a
controller is not resolved to `app/models/`, and references and rename do not
cross files. Names are matched by spelling, so two variables called `count` in
two functions count as one name. The server is for `.sl` files; `.slv`
templates get no diagnostics from it.

## Requirements

- The `soli` binary on your `PATH` (`soli --version` should work), or its
  absolute path in the editor's settings.
- Files with the `.sl` extension.
- A `soli.toml` at the project root, so editors find the workspace.

## VS Code & Cursor

The extension in `editors/vscode/` gives syntax highlighting and runs
`soli lint` on a file when it is opened and saved, showing the findings as
diagnostics; **Soli: Lint Current File** runs it on demand. It does not start
the language server yet, so hover, completion, go-to-definition, rename and
formatting are not available in these editors.

Install the packaged extension:

```bash
code --install-extension editors/vscode/soli-language-0.2.0.vsix
# Cursor:
cursor --install-extension editors/vscode/soli-language-0.2.0.vsix
```

or build it with `vsce package` from `editors/vscode/`.

Settings (`settings.json`):

| Setting | Default | |
|---|---|---|
| `soli.lint.enable` | `true` | Lint Soli files. |
| `soli.lint.onSave` | `true` | Lint again on every save. |
| `soli.lint.executablePath` | `"soli"` | The `soli` binary to run. |

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
   completion should activate within a second.

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
to a `soli` filetype first, and `nvim-lspconfig` has no `soli` entry, so
declare the server yourself:

```lua
-- ~/.config/nvim/after/plugin/soli.lua (or anywhere in your config)
vim.filetype.add({ extension = { sl = "soli" } })

vim.lsp.config("soli", {
  cmd = { "soli", "lsp" },
  filetypes = { "soli" },
  root_markers = { "soli.toml", ".git" },
})

vim.lsp.enable("soli")
```

Open a `.sl` file and `:checkhealth vim.lsp` lists the `soli` client. With
Neovim's default mappings, `K` hovers, `CTRL-]` jumps to the definition, `grr`
lists references, `grn` renames and `gra` offers the code actions; format with
`:lua vim.lsp.buf.format()`.

## Other editors

Any editor with an LSP client can start `soli lsp` on stdio for `.sl` files.
The shape of the entry, in the style most clients use:

```json
{
  "name": "soli",
  "command": ["soli", "lsp"],
  "filetypes": ["soli"],
  "fileExtensions": [".sl"],
  "rootPatterns": ["soli.toml"],
  "languageId": "soli"
}
```
