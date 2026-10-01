# Native Kernels

When a script loads, Soli compiles some of its functions to machine code: top-level functions typed
`Int`, `Float` or `Bool` end to end, whose bodies only do arithmetic, comparisons, branches, loops
and calls to other such functions. Everything else runs exactly as before, on the tree-walker or
the bytecode VM. Nothing has to be turned on, and the results are the same either way — including
the errors.

```soli
def fib(n: Int) -> Int
  return n if n < 2

  fib(n - 1) + fib(n - 2)
end

print(fib(32))   # 2178309
```

`fib` above is compiled with [Cranelift](https://cranelift.dev) before the first line runs. It
takes about a millisecond for a script's functions (0.4–1.2 ms measured), on both engines (`soli script.sl`,
which runs on the bytecode VM unless the script needs the tree-walker, `soli --tree script.sl` and
`soli --vm script.sl`).

## What it changes

Measured on Linux x86-64, total wall time of the process (median of 5), 3–4 ms of which is
start-up. "Before" is Soli 2.10.0, which had no kernels:

| | tree-walker before | `--vm` before | with kernels | Ruby 4.0.7 | Ruby `--yjit` |
|---|---:|---:|---:|---:|---:|
| `fib(32)` | 2259 ms | 491 ms | **12 ms** | 162 ms | 53 ms |
| a 20M-iteration Float loop in a function | 6156 ms | 1387 ms | **23 ms** | 601 ms | 599 ms |

The loop is:

```soli
def run(n: Int) -> Float
  sum = 0.0
  i = 0
  while i < n
    sum = sum + (i % 7) * 1.5
    i = i + 1
  end
  sum
end

print(run(20000000))   # 89999995.5
```

This covers numeric code only. A function that touches a string, an array, a hash, an object or a
builtin is not compiled, and runs at the speed it always did.

A call to a small kernel — up to about the size of a loop with a few statements; `fib` qualifies —
is compiled as a copy of the callee's body inside the caller, one level deep. The early exit
`return n if n < 2` then costs a branch instead of a call: `fib(42)` went from 1453 ms to 890 ms
with this alone. The copy checks the call depth and declines exactly as the call would have.

With kernels off (`SOLI_NATIVE=0`), `fib(32)` takes about 500 ms on today's VM against 486 ms in
2.10.0: a script on the VM now checks a function's declared return type on every return, as the
tree-walker always has, and that check costs about 3%. Without the `-> Int`, the same function
takes 488 ms, as before.

## Compared with Rust, C and Spinel

`fib(40)` with `n` read from the command line, so no compiler can work it out in advance. Best of
5, start-up included, same machine:

| | `fib(40)` | |
|---|---:|---|
| C, `gcc -O2` | 95 ms | one of the two calls becomes a loop (see below) |
| Rust `-O3`, unchecked arithmetic | 181 ms | overflow wraps around silently |
| **Rust `-O3`, checked arithmetic** | **360 ms** | `checked_add`/`checked_sub`, as Soli checks |
| **Soli** (a `soli build` executable) | **344 ms** | |
| Spinel (Ruby compiled to C) | 4 ms | gcc changes the algorithm — see below |

**Soli is on par with Rust once both check for overflow.** A kernel raises `integer overflow` where
release-mode Rust silently wraps around; each check is a compare and a branch, and that, not the
calls, is the gap to unchecked Rust. It is a guarantee of the language, so the kernel keeps it.

**C and Spinel do less work, not the same work faster.** gcc at `-O2` rewrites `fib`:

- Without an overflow check, it turns the second recursive call into a loop with an accumulator:
  about half the calls disappear.
- With one — Spinel always emits it, because Ruby raises on overflow — that rewrite is no longer
  allowed, and gcc takes a stronger route instead. It proves `fib` *pure* (no side effects, the
  result depends only on `n`: the `ipa-pure-const` pass), inlines several levels of the
  recursion, and computes once the calls that now appear more than once — `fib(n-3)`, `fib(n-4)`…
  It is memoization over a few levels, done by the compiler. The work drops from about 1.62ⁿ calls
  to about 1.25ⁿ: `fib(54)` takes 31 ms, where the recursion as written takes over a minute even
  in C.

The second route is easy to reproduce in plain C:

| `fib(42)`, plain C, `gcc -O2` | |
|---|---:|
| without an overflow check | 243 ms |
| **with** `__builtin_add_overflow` and `abort` | **2 ms** |
| the same, `-fno-ipa-pure-const` | 216 ms |

So on `fib`, Spinel's number measures gcc's reasoning about a pure function, not the cost of a call
— as Spinel's own README says of it. Kernels are pure by construction too, and could merge repeated
calls the same way; that pays off only for recursion whose sub-calls overlap, which real code
rarely has and memoizes explicitly when it does.

## Which functions qualify

A function is compiled when all of the following hold:

- It is a **top-level `def`** (also `export def`). Methods, nested functions and lambdas are not
  compiled.
- **Every parameter is annotated `Int`, `Float` or `Bool`**, with no default value and no block
  parameter; there is at least one and at most 16.
- **The return type is annotated `Int`, `Float` or `Bool`.**
- **The name is bound once**: defined by one `def`, and not also assigned elsewhere at the top
  level (`fib = 3`, `catch fib`, a `for fib in …`).
- The body only uses:
  - local variables — bare `x = …` or `let x: Int = …` — and the parameters, which can be
    reassigned;
  - `Int`, `Float` and `Bool` literals;
  - `+ - * / %`, the comparisons `== != < <= > >=`, `!`, unary `-`, `&&` and `||`;
  - `if` / `elsif` / `else`, `unless`, their postfix forms, and the ternary `c ? a : b`;
  - `while` with `break`;
  - `+= -= *= /= %=`;
  - `return`, postfix `return … if …`, and the implicit return of the last expression;
  - calls to other functions that qualify — recursion and mutual recursion included.

```soli
def gcd(a: Int, b: Int) -> Int
  while b != 0
    let remainder: Int = a % b
    a = b
    b = remainder
  end
  a
end

def clamp(x: Float, low: Float, high: Float) -> Float
  return low if x < low
  return high if x > high

  x
end

def is_even(n: Int) -> Bool
  return true if n == 0

  is_odd(n - 1)
end

def is_odd(n: Int) -> Bool
  return false if n == 0

  is_even(n - 1)
end
```

Anything else in the body — `print`, a string, an array, a method call, a closure, a global
variable, `try`, a `for` loop, a class, `nil` — and the function is not compiled. It is not an
error: the function runs as it always did. Three more rules:

- **A local keeps one type.** `x = 1` then `x = 2.5` in the same function is refused (the type
  checker refuses it too).
- **A bare assignment to a global name is refused.** If `total` is a top-level variable, a
  function, or a builtin, then `total = 0` inside a function writes *that* global on both engines;
  a kernel would make it a local, so the function is left alone. Rename the local.
- **A variable first assigned inside an `if` or a `while` belongs to that block.** Reading it after
  the block is refused: the tree-walker raises *Undefined variable* there while the VM reads it, and
  a kernel cannot agree with both. Assign it before the block (`result = 0`).

## Same results, same errors

A kernel either returns the value or **declines**, and when it declines the engine runs the call
itself, as if there were no kernel. Kernels have no side effects, so running the call again changes
nothing. Every error is therefore raised by the engine, with the message and line it always had:

- integer overflow (`*`, `+`, `-`, and `-x` when `x` is the smallest Int);
- division by zero, and `-9223372036854775808 / -1`;
- the call-depth limit (256 frames in a release build);
- a path through the function that returns no value;
- an argument of another type than the annotation says — the engines do not check parameter
  types, so `half(3)` on `def half(x: Float) -> Float` runs interpreted, as before;
- a call to a function that is not defined yet, or was redefined since.

A long native loop that overflows at its last iteration is replayed from the start by the engine
to raise the error. That is only the slow path on the way to an error.

Two behaviors worth knowing, because they are the language's and kernels keep them: **every Float
is truthy**, `0.0` included (`0` is falsy; the type checker refuses a Float condition anyway), and
**`&&` / `||` return one of their operands**, not a `Bool` (`3 && 5` is `5`).

## Write the hot loop inside the function

Each call from interpreted code into a kernel crosses a boundary: the engine packs the arguments,
calls the machine code and unpacks the result. Calls between kernels are direct machine calls. So
the loop belongs inside the typed function:

```soli
# Slow: the loop is interpreted, and each iteration crosses into a kernel.
def step(sum: Float, i: Int) -> Float
  sum + (i % 7) * 1.5
end

sum = 0.0
i = 0
while i < 2000000
  sum = step(sum, i)
  i += 1
end
```

On the machine above, those 2 million calls take 0.28 s on the VM (0.42 s with no kernels; 0.59 s
and 0.98 s on the tree-walker); `run(20000000)`, with ten times the iterations inside the function,
takes 23 ms.

## Seeing what was compiled

`SOLI_NATIVE_LOG=1` prints to stderr which functions were compiled, which were refused and why,
and the first decline of each kernel:

```soli
total = 0

def fib(n: Int) -> Int
  return n if n < 2

  fib(n - 1) + fib(n - 2)
end

def add_up(n: Int) -> Int
  total = 0
  i = 0
  while i < n
    total += i
    i += 1
  end
  total
end

def shout(n: Int) -> Int
  print("n = #{n}")
  n
end

def grow(n: Int) -> Int
  x = 1
  i = 0
  while i < n
    x *= 3
    i += 1
  end
  x
end

print(fib(20))
print(grow(100)) rescue print("overflow")
```

```text
$ SOLI_NATIVE_LOG=1 soli log_demo.sl
native: add_up (line 9) refused — assigns `total`, which names a global (line 10)
native: shout (line 19) refused — calls `print` (line 20)
native: fib(Int) -> Int compiled (line 3)
native: grow(Int) -> Int compiled (line 24)
native: 2 kernel(s) compiled in 0.60 ms
6765
native: grow declined (integer overflow) — running it interpreted (logged once)
overflow
```

`add_up` is refused because `total = 0` writes the top-level `total`; `grow(100)` overflows, so
the kernel declines and the engine (the VM, for this script) raises the overflow, which `rescue`
catches.

`SOLI_NATIVE_LOG=2` also prints the Cranelift IR of each kernel.

## Where it applies

Kernels are compiled for scripts, on whichever engine runs them: `soli script.sl`,
`soli --tree script.sl`, `soli --vm script.sl`, `soli -e "…"`, and
executables built with `soli build script.sl` (see [Deployment](deploy.md#script-executables)), which
compile them when they start, on the machine they run on.

They are not compiled, yet, under `soli serve`, `soli test`, the REPL, or a coverage run.

## Turning it off

```bash
SOLI_NATIVE=0 soli script.sl     # also off, false, no
soli --no-native script.sl
```

If the operating system forbids executable memory (SELinux `deny_execmem`, a hardened macOS
runtime), the log says so and everything runs interpreted. A `soli` built from source without the
`native` Cargo feature has no kernels at all.

## See also

- [Functions](soli-language.md#functions) — declarations, type annotations, return values.
- [Configuration](configuration.md#native-kernels) — `SOLI_NATIVE`, `SOLI_NATIVE_LOG`.
- [Internals: native kernels](internals/native.md) — how the tier is built, for contributors.
