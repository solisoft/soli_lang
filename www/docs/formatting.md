# Formatting (`soli fmt`)

`soli fmt` rewrites `.sl` source files into Soli's canonical style. It parses
the file, then re-emits the AST with normalized indent, spacing, and keywords —
so the output is guaranteed to be syntactically identical to the input, just
re-formatted.

## Usage

```bash
soli fmt                          # Format every .sl file under the cwd, in place
soli fmt path/to/file.sl          # Format a single file
soli fmt app/ lib/                # Format every .sl file under one or more paths
soli fmt --check                  # Don't rewrite — exit non-zero if any file would change
soli fmt --stdin < in.sl > out.sl # Filter mode for editor integration
```

A few details worth knowing:

- With no path argument, the current working directory is walked recursively.
- Without `--check`, every changed file is written back in place. Already-formatted
  files are left untouched (no spurious mtime bump).
- `--check` prints `would reformat: <path>` for each file that would change and
  exits with status `1` if any file is unformatted. Use this in CI to enforce
  style.
- `--stdin` reads from stdin and writes to stdout — no file is touched. This is
  the mode editor integrations should use to run a "format on save" or
  "format selection" command.

## Style

| Rule | Detail |
|------|--------|
| Indent | 2 spaces, never tabs |
| Functions | `def name() ... end` (parens kept even when empty) |
| Classes | `class X < Y ... end` |
| Control flow | `if cond ... end`, `while cond ... end`, etc. |
| Operators | Normalized spacing — `a + b`, `a == b`, `a && b` |
| Comments | Preserved at their original line positions |
| Comment marker | `//` line comments are normalized to `#` |
| Blank lines | Multiple consecutive blank lines collapse to one |
| Guard clauses | `if cond return … end` with no `else` gets a trailing blank line |

The formatter never changes program semantics. If you spot output that breaks
your code, that's a bug — please report it.

## Editor integration

Hook `soli fmt --stdin` into your editor's "format document" command. The
filter mode is the same shape as `gofmt`, `rustfmt --emit=stdout`, or
`black -`, so most LSP/format-tool wrappers can drive it with no extra config.

For LSPs that prefer a project command, point at:

```bash
soli fmt --stdin
```

See [Editor Integration](/docs/development-tools/editor-integration) for
language-server setup.

## CI usage

The recommended CI check:

```bash
soli fmt --check
```

This exits non-zero on the first unformatted file it sees, listing each one as
`would reformat: <path>`. Pair it with `soli lint` for a complete style gate.

## Coverage and the fallback

Soli's grammar has a handful of advanced forms the formatter does not yet
canonicalize: `@sdbql{ … }` query blocks, list/hash comprehensions, and some
complex match patterns. For these nodes the formatter copies the original
source bytes verbatim via the AST span — semantics are preserved, and the
surrounding code is still formatted, but the body of the un-modeled node is
left untouched.

You can mix formatted and un-modeled code freely; running `soli fmt` repeatedly
is safe (the output is a fixed point).

## See also

- [Linting](/docs/language/linting) — style and smell rules enforced by `soli lint`.
- [Editor Integration](/docs/development-tools/editor-integration) — wiring `soli fmt` and the LSP into your editor.
