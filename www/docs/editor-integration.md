# Editor Integration

Soli ships a Language Server (`soli lsp`) so any editor that speaks LSP can
offer hover, completion, go-to-definition, references, rename, format,
diagnostics, document symbols, folding ranges, inlay hints, and code actions.

## Requirements

- The `soli` binary must be on your `PATH` (`soli --version` should work).
- Files should use the `.sl` extension so the editor's language detection
  kicks in.
- A `soli.toml` at the project root helps editors detect the workspace.

## VS Code & Cursor

```bash
cd editors/vscode
vsce package
# Install the generated .vsix from your editor's command palette.
```

Settings:

```json
{
  "soli.lsp.enable": true,
  "soli.lint.onSave": true
}
```

## Nova (macOS)

The Nova extension lives under `editors/nova/soli.novaextension/` and uses
Nova's `LanguageClient` API to spawn `soli lsp` on stdio.

### Install from the bundled source

1. Build and install the `soli` binary so the LSP backend is available:
   ```bash
   cargo install --path .
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

Built-in LSP support, configured via `lspconfig`:

```lua
-- ~/.config/nvim/lua/lsp/soli.lua
local lspconfig = require('lspconfig')

lspconfig.soli.setup({
  cmd = {"soli", "lsp"},
  filetypes = {"soli"},
  root_dir = function(filename)
    return lspconfig.util.root_pattern("soli.toml", ".git")(filename)
  end,
  capabilities = require('cmp_nvim_lsp').default_capabilities(),
})
```

Or, with Neovim 0.10+'s native config:

```lua
vim.lsp.config('soli', {
  cmd = {"soli", "lsp"},
  filetypes = {"soli"},
  root_markers = {"soli.toml"},
})

vim.lsp.enable('soli')
```

## Other editors

Any LSP-aware editor can wire `soli lsp` up directly:

```json
{
  "name": "soli",
  "command": "soli lsp",
  "filetypes": ["soli"],
  "rootPatterns": ["soli.toml"],
  "languageId": "soli"
}
```

## LSP features

The Soli LSP currently advertises these capabilities:

- `hover` — type/kind info and builtin docs
- `completion` — keywords, types, in-scope symbols
- `definition`, `typeDefinition`, `references`, `rename`
- `documentSymbol`, `foldingRange`, `inlayHint`
- `formatting`, `rangeFormatting`
- `codeAction` — quick-fixes for lint violations
- diagnostics streamed from `soli lint`
