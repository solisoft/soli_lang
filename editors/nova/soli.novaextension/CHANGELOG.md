# Changelog

## Unreleased

- `.slv` view templates are declared as the **Soli Template** language and sent
  to `soli lsp`, which lints their `<% %>` code (it needs a `soli` with #691).
  Buffers in Nova's `html` syntax are sent too, in case `.slv` opens under it;
  the server ignores every file that is not `.sl` or `.slv`.

## 0.1.0

- Initial release.
- TextMate grammar shared with the VS Code extension.
- LSP integration backed by `soli lsp`: hover, completion, go-to-definition,
  references, rename, formatting, document symbols, folding ranges, inlay
  hints, code actions, diagnostics.
