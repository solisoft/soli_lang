# Soli for Nova

[Soli](https://github.com/solisoft/soli_lang) language support for the [Nova](https://nova.app) editor.

## Features

- Syntax highlighting (TextMate grammar shared with the official VS Code extension).
- Language Server integration via `soli lsp` (stdio):
  - Hover with type/kind info
  - Auto-completion (keywords, types, locals)
  - Go-to-definition
  - Find references
  - Rename symbol
  - Document & range formatting
  - Document symbols (outline)
  - Folding ranges
  - Inlay hints
  - Code actions (quick-fixes for lint rules)
  - Diagnostics from `soli lint`

## Requirements

The extension shells out to the `soli` binary that ships the LSP server. Install
it once and Nova picks it up automatically:

```bash
# From the soli_lang repo:
cargo install --path .
# Or, if you already have a release build:
brew install solisoft/tap/soli   # (when available on your platform)
```

`soli --version` must work in the same shell Nova inherits — if it doesn't,
set the **Soli > Soli binary** path in the extension's preferences to the
absolute path of your `soli` executable.

## Settings

| Setting | Description |
|---|---|
| `soli.lsp.enabled` | Toggle the LSP server. When off, you still get TextMate highlighting. |
| `soli.lsp.path` | Absolute path to `soli`. Leave blank to use PATH. |
| `soli.lsp.trace` | LSP trace verbosity (`off`/`messages`/`verbose`). Output goes to the extension console. |

## Commands

- **Editor → Extensions → Restart Soli Language Server** — handy when picking
  up a freshly rebuilt `soli` binary without reloading Nova.

## Development

To iterate on the extension locally:

1. Symlink the bundle into Nova's extensions directory:
   ```bash
   ln -s "$PWD/soli.novaextension" \
     "$HOME/Library/Application Support/Nova/Extensions/com.solilang.soli.novaextension"
   ```
2. Launch Nova in development mode (`Extensions → Activate Project as Extension`)
   or use the Extension Library's developer reload.
3. Make sure `cargo install --path .` was run after editing the LSP server, then
   invoke **Restart Soli Language Server** to pick up the new binary.

## License

MIT — same as the Soli language.
