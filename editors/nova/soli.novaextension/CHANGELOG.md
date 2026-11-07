# Changelog

## 0.2.0 (unreleased)

- The grammar is now a copy of the canonical `editors/grammars/soli.tmLanguage.json`
  (`scripts/sync-grammars.sh`), the same one the VS Code extension ships. Its
  keywords match the lexer (`module`, `enum`, `self`, `nil`, `begin`, `ensure`,
  `rescue`, `not`, `and`, `or`, `do`, `then`, `Decimal` added; `async`,
  `await`, `Any` removed), the removed `[[ … ]]` strings and `\U########`
  escape are gone, and `"""…"""`, `r"…"`, backtick strings, `#{}` interpolation,
  `@ivar`, `@sdbql{…}`, symbols, `%w[]`/`%i[]`/`%n[]` and `19.99D` are highlighted.
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
