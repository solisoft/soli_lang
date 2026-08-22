# Pipeline — lexer, parser, AST

![Source pipeline](/images/internals/source-pipeline.jpg)

Every Soli file goes through the same front-end. Engines only diverge **after** a `Program` exists.

```
source: &str
    → Scanner::new(source).scan_tokens()  → Vec<Token>
    → Parser::new(tokens).parse()         → Program
    → optional TypeChecker::check
    → optional ModuleResolver::resolve    (imports)
    → Interpreter::interpret  or  Compiler::compile + Vm::run
```

You can see this literally in `src/lib.rs` (`run_with_path`, `run_vm`).

## Spans

`src/span.rs` — `Span { start, end, line, column }`. Almost every AST node and every `Token` carries one. Errors (`LexerError`, `ParseError`, `RuntimeError`) use it to print `file:line:col`. When you add a node, **copy the span from the token**; a `Span::default()` makes the diagnostic land on line 1.

---

## Lexer (`src/lexer/`)

| Type | File | Role |
|---|---|---|
| `Scanner<'a>` | `scanner.rs` | Walks `&str`, emits tokens |
| `Token` | `token.rs` | `kind` + `lexeme` + `span` |
| `TokenKind` | `token.rs` | The enum of all tokens |
| `SdqlInterpolation` | `token.rs` | `#{expr}` inside `@sdbql{…}` |

### `Scanner`

```rust
pub struct Scanner<'a> { /* source, current, line, … */ }

impl Scanner<'_> {
    pub fn new(source: &str) -> Self
    pub fn scan_tokens(&mut self) -> Result<Vec<Token>, LexerError>
    pub fn scan_token(&mut self) -> Result<Token, LexerError>  // one token
}
```

`scan_tokens` loops `scan_token` until `Eof`. Keywords are matched after an identifier is scanned (so `class` is `TokenKind::Class`, not `Identifier("class")`).

Interpolation (`"Hello #{name}"`), raw strings (`[[…]]`, `"""…"""`, `r"…"`), SDBQL blocks, and percent arrays (`%w[a b]`) are lexer features, not parser tricks.

### `Token` / `TokenKind`

`TokenKind` is large: literals (int/float/decimal/string/bool/symbol), every keyword (`Let`, `Class`, `Match`, …), operators, and special forms (`SdqlBlock`, `InterpolatedString`).

If you add syntax:

1. Add a `TokenKind` variant (or reuse `Identifier` + a keyword table).
2. Teach `Scanner` to emit it.
3. Teach the parser to consume it.
4. Add an AST node if the meaning is new.

---

## Parser (`src/parser/`)

| File | Role |
|---|---|
| `core.rs` | `Parser` struct, `new`, `parse`, token cursor (`advance`, `check`, `match_token`) |
| `declarations.rs` | `class`, `def`, `module`, fields |
| `statements.rs` | `if`, `while`, `for`, `return`, assignment |
| `expressions.rs` | Pratt / precedence climbing |
| `precedence.rs` | Operator table |
| `types.rs` | Type annotations in the grammar |

```rust
pub struct Parser { /* tokens, current */ }

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self
    pub fn parse(&mut self) -> ParseResult<Program>
}
```

`parse()` returns `Program { statements: Vec<Stmt> }`. It is a **recursive descent** parser with a precedence table for binary operators (same idea as Crafting Interpreters).

Junior trap: the parser is split across files as `impl Parser` blocks. `self.expression()` in `statements.rs` is the same `Parser`. Search for `fn expression` rather than assuming one file owns it.

---

## AST (`src/ast/`)

| File | Types |
|---|---|
| `expr.rs` | `Expr`, `ExprKind`, `Argument`, `NamedArgument` |
| `stmt.rs` | `Stmt`, `StmtKind`, `Program`, class/fn declarations |
| `types.rs` | `TypeAnnotation` as parsed (not the checker’s interned types) |

`Expr` / `Stmt` are **boxes around an enum + a Span**:

```rust
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}
```

Important `ExprKind` families (not exhaustive):

- Literals: `IntLiteral`, `FloatLiteral`, `StringLiteral`, `BoolLiteral`, `Null`, `Symbol`
- Names: `Variable`, `This`, `Super`
- Operators: `Binary`, `Unary`, `Assign`
- Calls: `Call { callee, arguments }`, `Pipeline`
- Access: `Member`, `SafeMember`, `Index`, `QualifiedName`
- Collections: array/hash literals, comprehensions
- Functions: lambdas
- Special: `CommandSubstitution`, `SdqlBlock`, `InterpolatedString`

`Argument` is `Positional(Expr) | Named(NamedArgument) | Block(Expr)` — named params and `do` blocks are first-class in the tree.

The tree-walker **matches on `ExprKind`**. The VM **compiles** each variant to `Op`s. A new `ExprKind` without a compiler arm becomes an `EngineFallback` (handler silently runs on the interpreter in production after a failed compile). Prefer implementing both.

---

## Type checker (`src/types/`) — optional

`soli check` calls `TypeChecker::check`. Annotations are optional; unannotated code is mostly `Any`. Controllers that `render` / `redirect` are notoriously hard to check — that is known, not a bug in your change.

| File | Role |
|---|---|
| `type_repr.rs` | Internal type representations |
| `environment.rs` | Type scopes |
| `checker/mod.rs` | `TypeChecker` |
| `checker/expressions/` | Per-expression checking |

---

## Modules (`src/module/resolver.rs`)

`import "./foo.sl"` is not handled by the parser as “load file”. The parser produces an import statement; `ModuleResolver::resolve(program, path)` splices imported declarations into the program. `run_with_path` only runs the resolver when the file has imports.

---

## Errors

| Type | When |
|---|---|
| `LexerError` | Bad character, unterminated string |
| `ParseError` | Unexpected token |
| `RuntimeError` | Division by zero, missing method, user `throw` |
| `SolilangError` | Wrapper enum used by `lib.rs` |

`From` impls let `?` bubble from scan → parse → interpret.
