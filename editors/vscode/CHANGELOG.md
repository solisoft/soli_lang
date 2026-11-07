# Changelog

## 0.3.0 (unreleased)

- **Language server.** The extension starts `soli lsp` and gets hover,
  completion (members after `.` and `@`), project-wide go-to-definition,
  references, rename, formatting, document symbols, folding, code actions and
  diagnostics, path-aware lint rules included. `.slv` templates are sent too,
  for their diagnostics and completion inside `<% %>`.
- **Finding `soli`.** `soli.lsp.path`, then `PATH`, then `~/.local/bin`,
  `~/.cargo/bin` and `/usr/local/bin` (`soli.exe` in `%USERPROFILE%\.cargo\bin`
  and `%USERPROFILE%\.local\bin` on Windows). When nothing is found, a
  notification lists where it looked.
- **Lint fallback.** `soli lint` on open and save runs only while the language
  server does not: disabled with `soli.lsp.enabled`, a `soli` built without
  the `lsp` feature (detected, and not restarted), or a server that crashed
  five times in three minutes.
- **Settings** follow the Nova extension: `soli.lsp.enabled`, `soli.lsp.path`,
  `soli.lsp.trace`. `soli.lint.executablePath` is deprecated and read only when
  `soli.lsp.path` is empty. Both path settings are user settings only
  (machine scope): a workspace's `.vscode/settings.json` cannot pick the
  binary that runs. The extension is off in untrusted workspaces.
- **Commands:** *Soli: Restart Language Server*, *Soli: Show Output*,
  *Soli: Lint Current File*.
- **Grammar** regenerated from `editors/grammars/`: the keywords now match the
  lexer (`module`, `enum`, `self`, `nil`, `begin`, `ensure`, `rescue`, `not`,
  `and`, `or`, `do`, `then`, `Decimal` added; `async`, `await`, `Any` removed),
  plus `#{}` interpolation, `@ivar`, `r"…"`, backtick strings, `@sdbql{…}`,
  symbols, `%w[]`/`%i[]`/`%n[]`, `19.99D`, `?`/`!` method names.
- **`.slv` templates** are their own language, *Soli Template*: HTML with
  Soli highlighted inside `<% %>`, `<%= %>`, `<%- -%>`, and `<%# %>` comments.
- Requires VS Code 1.82 or later (Cursor, VSCodium and Windsurf builds based on
  it).

## 0.2.0

- `soli lint` on open and save, shown as diagnostics; *Soli: Lint Current File*.

## 0.1.0

- Initial release: TextMate grammar for `.sl` files.
