# Error Messages and Error Codes

When `soli` stops on an error, it says what went wrong, shows the line, and
points at the place. Most kinds of error also carry a **stable code** —
`E0301` is an undefined variable in every release — and `soli explain`
prints what the code means and how to fix it.

## Reading a code frame

```soli
# report.sl
name = "Alice"
print(nme)
```

```text
$ soli report.sl
Error: Type error: Undefined variable 'nme' (did you mean 'name'?) at 3:7
 --> report.sl:3:7
  |
3 | print(nme)
  |       ^^^
  |
  = help: did you mean 'name'?
  = note: error[E0301] undefined-variable — `soli explain E0301`
```

- The **first line is unchanged**: `Error: <message> at line:column`, as it
  always read. A script or CI step that greps it keeps working; the frame is
  added below it.
- ` --> file:line:column` names the file. In a program with imports, an error
  raised inside an imported module is framed in **that module**, not in the
  file you ran.
- The underline covers the name or token at the error's column. Both engines
  draw the same frame: `soli x.sl` and `soli --tree x.sl` print the same
  error.
- `= help:` repeats the "did you mean" suggestion, when there is one.
- `= note:` names the code and the command that explains it.

The same frame appears under `soli -e`, under `soli check` (one per error),
and under `soli lint --frames`. Colour is used only when the output is a
terminal, and never when `NO_COLOR` is set.

An error whose place is not known for certain prints its first line only.
A frame drawn from the wrong file would mislead more than no frame, so `soli`
names a file only when the engine or the import resolver said where the error
came from, or the program is a single file, or the error's position agrees
with exactly one of the program's files.

## `soli explain`

```bash
soli explain E0301                  # by code
soli explain undefined-variable     # by name
soli explain smell/undefined-local  # a lint rule
soli explain                        # list every code and rule
```

An unknown code exits with status `1` and suggests the closest one
(`Did you mean 'E0301'?`).

## Machine-readable output

`soli check --format json` and `soli lint --format json` print one JSON
document instead of text, for editors and CI:

```json
{
  "diagnostics": [
    {
      "code": "E0301",
      "column": 7,
      "file": "report.sl",
      "hint": "did you mean 'name'?",
      "line": 3,
      "message": "Undefined variable 'nme'",
      "name": "undefined-variable",
      "notes": [],
      "secondary": [],
      "severity": "error"
    }
  ],
  "errors": 1,
  "files_checked": 1,
  "warnings": 0
}
```

For a lint finding, `code` is the rule id (`smell/undefined-local`) and
`name` is `null`; `soli lint` prints `files_linted` and `issues` instead of
the counts above. The exit status is the same as in text mode. The editor
extension (LSP) reports the same codes on syntax errors.

## The codes

Codes are never renumbered or reused: a retired code stays reserved. An error
that is still a free-form message has no code yet, rather than a catch-all one.

| Range | What |
|---|---|
| `E01xx` | the lexer |
| `E02xx` | the parser |
| `E03xx` | names and scopes |
| `E04xx` | types and calls, found by `soli check` or at run time |
| `E05xx` | other run-time failures |
| `E06xx` | the framework |
| `E07xx` | imports |
| `W0xxx` | warnings |

| Code | Name | Reported as | Meaning |
|---|---|---|---|
| `E0101` | `unexpected-character` | syntax error | a character that starts no token (`$`) |
| `E0102` | `unterminated-string` | syntax error | a string with no closing quote |
| `E0103` | `invalid-escape` | syntax error | `\q` and other unknown escapes |
| `E0104` | `invalid-number` | syntax error | an integer past 64 bits, `12D` with no decimals |
| `E0201` | `unexpected-token` | syntax error | the parser expected one token and found another |
| `E0202` | `unexpected-end-of-file` | syntax error | a `def`, block or bracket never closed |
| `E0203` | `invalid-assignment-target` | syntax error | `add(1) = 2` |
| `E0301` | `undefined-variable` | `NameError` | a name no scope defines |
| `E0302` | `undefined-function` | `soli check` | reserved — an unknown function reports as `E0301` today |
| `E0303` | `undefined-type` | `soli check` | a typed `match` pattern naming an unknown type |
| `E0304` | `unknown-member` | `NoMethodError` | a method or field the value does not have |
| `E0305` | `this-outside-class` | `soli check` | `this` with no instance |
| `E0306` | `super-outside-class` | `soli check` | `super` outside a method |
| `E0307` | `no-superclass` | `soli check` | `super` in a class with no parent |
| `E0401` | `type-mismatch` | `TypeError` | a value of the wrong type |
| `E0402` | `not-callable` | `TypeError` | calling something that is not a function |
| `E0403` | `wrong-arity` | `ArgumentError` | too many or too few arguments |
| `E0501` | `division-by-zero` | `ZeroDivisionError` | integer `/` or `%` by zero |
| `E0502` | `index-out-of-bounds` | `IndexError` | an index past the end |
| `E0503` | `not-a-class` | `TypeError` | a superclass or `new` target that is not a class |
| `E0504` | `uncaught-exception` | the thrown class, or `Error` | a `throw` nothing caught |
| `E0601` | `record-not-found` | `404` in a request | `Model.find` found nothing |
| `E0602` | `forbidden` | `403` in a request | `forbidden()` or a policy refused |
| `E0701` | `module-not-found` | import error | an `import` of a file that does not exist |
| `E0702` | `circular-import` | import error | files importing each other |
| `E0703` | `import-error` | import error | an unexported name, or an absolute path |
| `E0704` | `module-syntax-error` | import error | an imported file that does not parse |
| `W0401` | `non-exhaustive-match` | `soli check` warning | a `match` on an enum missing variants |

Lint rules keep their own ids (`smell/undefined-local`, `idiom/prefer-blank`,
…); see [`linting.md`](linting.md). `soli explain` takes them too.

## See also

- [`debugging.md`](debugging.md) — breakpoints, the REPL, the dev bar
- [`linting.md`](linting.md) — `soli lint` and `soli check`
- Rendered page: `/docs/development-tools/error-codes`
