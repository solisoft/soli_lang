# Soli for VS Code, Cursor, VSCodium and Windsurf

Language support for [Soli](https://soli.solisoft.net), the Ruby-like
language and web framework: the `soli lsp` language server, and highlighting
for `.sl` files and `.slv` view templates.

## Features

- **Diagnostics** while you type: syntax errors, and the `soli lint` rules,
  including the path-aware ones for `app/controllers` and `app/services`
  (`security/unfiltered-mass-assignment`, `style/redundant-model-import`…).
  `.slv` templates have their `<% %>` code checked.
- **Hover**, **completion** (the receiver's members after `.` and `@`),
  **go to definition** across the application, **references**, **rename**,
  **document symbols**, **folding**, **code actions**, and **Format Document**
  with the same formatter as `soli fmt`.
- **Highlighting** for Soli (`#{}` interpolation, `@ivars`, `@sdbql{…}`
  blocks, symbols, raw and triple-quoted strings) and for `.slv` templates
  (HTML with Soli inside `<% %>`, `<%= %>` and `<%- -%>`).

Everything but highlighting comes from the `soli` binary: install it first.

```bash
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh
```

## Finding `soli`

The extension runs, in order: the `soli.lsp.path` setting; `soli` on `PATH`;
then `~/.local/bin/soli`, `~/.cargo/bin/soli` and `/usr/local/bin/soli`
(on Windows `soli.exe` in `%USERPROFILE%\.cargo\bin` and
`%USERPROFILE%\.local\bin`). An editor started from the dock or the start
menu often lacks your shell's `PATH`, which is why those directories are
tried. If nothing is found, a notification lists every place it looked.

## Settings

| Setting | Default | |
|---|---|---|
| `soli.lsp.enabled` | `true` | Run the language server. Off: highlighting, plus `soli lint` on open and save. |
| `soli.lsp.path` | `""` | Absolute path to `soli`. Empty: `PATH`, then the directories above. |
| `soli.lsp.trace` | `"off"` | `messages` or `verbose` traces the LSP traffic in the **Soli** output. |
| `soli.lint.enable` | `true` | Run `soli lint` when the language server is not running. |
| `soli.lint.onSave` | `true` | Lint again on each save (fallback lint only). |
| `soli.lint.executablePath` | `""` | Deprecated: read only when `soli.lsp.path` is empty. |

The names match the Nova extension's. `soli.lsp.path` and
`soli.lint.executablePath` are read from user settings only, so a
repository's `.vscode/settings.json` cannot make the editor run another
binary; the extension stays off in an untrusted workspace.

## When the language server cannot run

`soli lint` takes over, on open and save, when the server is disabled, when
the `soli` found was built without the `lsp` feature (detected from its
message, and not restarted), or when the server stopped five times in three
minutes. It never runs while the server does, so a finding is reported once.

## Commands

- **Soli: Restart Language Server** — picks up a rebuilt `soli`.
- **Soli: Show Output** — the server's log, and the trace.
- **Soli: Lint Current File** — the fallback lint, on demand.

## Building from source

```bash
cd editors/vscode
npm ci
npm test            # unit and grammar tests
npm run package     # writes soli-language.vsix
code --install-extension soli-language.vsix   # or: cursor --install-extension …
```

The grammars are generated: edit `editors/grammars/*.tmLanguage.json` and run
`scripts/sync-grammars.sh`.
