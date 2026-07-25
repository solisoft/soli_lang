# Cross-language benchmark: Soli side. Output: category|name|best_ms
def bench(cat, name, work) {
    work()
    let bestms = 999999.0
    let r = 0
    while r < 7
        let t0 = clock()
        work()
        let t1 = clock()
        let ms = (t1 - t0) * 1000.0
        if ms < bestms { bestms = ms }
        r = r + 1
    end
    print("#{cat}|#{name}|#{bestms}")
}
const N = 20000
def mk_ints(n) {
    let a = []
    let i = 0
    while i < n { a.push(i); i = i + 1 }
    return a
}
def mk_strs(n) {
    let a = []
    let i = 0
    while i < n { a.push("item-#{i}"); i = i + 1 }
    return a
}
def mk_hash(n) {
    let h = {}
    let i = 0
    while i < n { h["key-#{i}"] = i; i = i + 1 }
    return h
}
def mk_words(n) {
    let a = []
    let i = 0
    while i < n { a.push("word#{i}"); i = i + 1 }
    return a.join(" ")
}
# Deterministic LCG so Soli and Ruby get byte-identical inputs.
def lcg(n) {
    let out = []
    let x = 12345
    let i = 0
    while i < n {
        x = (x * 1103515245 + 12345) % 2147483648
        out.push(x % 100000)
        i = i + 1
    }
    return out
}
def mk_range(lo, hi) {
    let a = []
    let i = lo
    while i < hi { a.push(i); i = i + 1 }
    return a
}
def mk_dups(n) {
    let a = []
    let i = 0
    while i < n { a.push(i % (n / 4)); i = i + 1 }
    return a
}
def mk_nested(n) {
    let outer = []
    let i = 0
    while i < n / 10 {
        let inner = []
        let j = 0
        while j < 10 { inner.push(i * 10 + j); j = j + 1 }
        outer.push(inner)
        i = i + 1
    }
    return outer
}
const A = lcg(N)
const B = mk_range(N / 2, N + N / 2)
const DUPS = mk_dups(N)
const NESTED = mk_nested(N)
const SA = mk_strs(N)
const H = mk_hash(N)
const G = mk_hash(N)
const S = mk_words(8000)

def h_get()     { let h = H; let i = 0; while i < N { h["key-500"]; i = i + 1 } }
def h_set()     { let h = H; let i = 0; while i < N { h["zz"] = i; i = i + 1 } }
def h_haskey()  { let h = H; let i = 0; while i < N { h.has_key("key-500"); i = i + 1 } }
def s_concat() {
    let r = ""
    let i = 0
    while i < 10000 { r = r + "x"; i = i + 1 }
    return r
}
def s_interp() {
    let i = 0
    let r = ""
    while i < N { r = "a#{i}b"; i = i + 1 }
    return r
}
def n_int() {
    let s = 0
    let i = 0
    while i < N { s = s + i; i = i + 1 }
    return s
}
def n_float() {
    let s = 0.0
    let i = 0
    while i < N { s = s + i * 1.5; i = i + 1 }
    return s
}
def n_mod() {
    let s = 0
    let i = 0
    while i < N { s = s + (i % 7); i = i + 1 }
    return s
}
def inc(x) { return x + 1 }
def c_closure() {
    let f = fn(x) { return x + 1 }
    let s = 0
    let i = 0
    while i < N { s = f(s); i = i + 1 }
    return s
}
def c_fn() {
    let s = 0
    let i = 0
    while i < N { s = inc(s); i = i + 1 }
    return s
}

bench("Array", "build",        fn() mk_ints(N))
bench("Array", "map",          fn() A.map(fn(x) x * 2))
bench("Array", "filter",       fn() A.filter(fn(x) x > 10))
bench("Array", "reduce",       fn() A.reduce(fn(a, x) a + x, 0))
bench("Array", "each",         fn() A.each(fn(x) x))
bench("Array", "sort",         fn() A.sort())
bench("Array", "reverse",      fn() A.reverse())
bench("Array", "uniq",         fn() DUPS.uniq())
bench("Array", "union",        fn() A.union(B))
bench("Array", "intersection", fn() A.intersection(B))
bench("Array", "difference",   fn() A.difference(B))
bench("Array", "flatten",      fn() NESTED.flatten())
bench("Array", "sum",          fn() A.sum())
bench("Array", "join",         fn() SA.join(","))
bench("Array", "includes",     fn() A.includes?(N - 1))
bench("Array", "index_of",     fn() A.index_of(N - 1))
bench("Hash", "build",         fn() mk_hash(N))
bench("Hash", "get",           fn() h_get())
bench("Hash", "set",           fn() h_set())
bench("Hash", "has_key",       fn() h_haskey())
bench("Hash", "keys",          fn() H.keys())
bench("Hash", "values",        fn() H.values())
bench("Hash", "merge",         fn() H.merge(G))
bench("Hash", "each",          fn() H.each(fn(k, v) v))
bench("Hash", "select",        fn() H.select(fn(k, v) v > 10))
bench("Hash", "transform_values", fn() H.transform_values(fn(v) v))
bench("Hash", "invert",        fn() H.invert())
bench("String", "upcase",      fn() S.uppercase())
bench("String", "downcase",    fn() S.lowercase())
bench("String", "split",       fn() S.split(" "))
bench("String", "chars",       fn() S.chars())
bench("String", "bytes",       fn() S.bytes())
bench("String", "replace_all", fn() S.replace_all("word", "W"))
bench("String", "sub",         fn() S.sub("word", "W"))
bench("String", "contains",    fn() S.contains("word7999"))
bench("String", "index_of",    fn() S.index_of("word7999"))
bench("String", "reverse",     fn() S.reverse())
bench("String", "capitalize",  fn() S.capitalize())
bench("String", "concat_plus", fn() s_concat())
bench("String", "interpolate", fn() s_interp())
bench("Numeric", "int_loop",   fn() n_int())
bench("Numeric", "float_math", fn() n_float())
bench("Numeric", "modulo",     fn() n_mod())
bench("Control", "closure_call", fn() c_closure())
bench("Control", "fn_call",    fn() c_fn())

# --- DateTime / Duration ---
const DT_ISO = "2026-01-01T00:00:00Z"
const T1 = DateTime.parse(DT_ISO)
const T2 = DateTime.parse("2026-03-15T12:00:00Z")
const M = 20000
def dt_now()      { let i = 0; while i < M { DateTime.now(); i = i + 1 } }
def dt_parse()    { let i = 0; while i < M { DateTime.parse(DT_ISO); i = i + 1 } }
def dt_format()   { let i = 0; while i < M { T1.format("%Y-%m-%d"); i = i + 1 } }
def dt_add()      { let i = 0; while i < M { T1.add_hours(5); i = i + 1 } }
def dt_sub()      { let i = 0; while i < M { T1.subtract_days(10); i = i + 1 } }
def dt_year()     { let i = 0; while i < M { T1.year(); i = i + 1 } }
def dt_eom()      { let i = 0; while i < M { T1.end_of_month(); i = i + 1 } }
def dt_fromunix() { let i = 0; while i < M { DateTime.from_unix(1700000000); i = i + 1 } }
def dt_tounix()   { let i = 0; while i < M { T1.to_unix(); i = i + 1 } }
def du_ofdays()   { let i = 0; while i < M { Duration.of_days(3); i = i + 1 } }
def du_between()  { let i = 0; while i < M { Duration.between(T1, T2); i = i + 1 } }
bench("DateTime", "now",           fn() dt_now())
bench("DateTime", "parse",         fn() dt_parse())
bench("DateTime", "format",        fn() dt_format())
bench("DateTime", "add_hours",     fn() dt_add())
bench("DateTime", "subtract_days", fn() dt_sub())
bench("DateTime", "year",          fn() dt_year())
bench("DateTime", "end_of_month",  fn() dt_eom())
bench("DateTime", "from_unix",     fn() dt_fromunix())
bench("DateTime", "to_unix",       fn() dt_tounix())
bench("Duration", "of_days",       fn() du_ofdays())
bench("Duration", "between",       fn() du_between())

# --- Field-keyed aggregates ---
def mk_rows(n) {
    let a = []
    let i = 0
    while i < n { a.push({"t": "type#{i % 7}", "n": i}); i = i + 1 }
    return a
}
def mk_flat(n) {
    let a = []
    let i = 0
    while i < n { a.push(i % 7); i = i + 1 }
    return a
}
const ROWS = mk_rows(N)
const FLAT = mk_flat(N)
bench("Aggregate", "sum_by",   fn() ROWS.sum_by("n"))
bench("Aggregate", "group_by", fn() ROWS.group_by("t"))
bench("Aggregate", "index_by", fn() ROWS.index_by("t"))
bench("Aggregate", "count_by", fn() ROWS.count_by("t"))
bench("Aggregate", "tally",    fn() FLAT.tally())
bench("Aggregate", "avg_by",    fn() ROWS.avg_by("n"))
bench("Aggregate", "uniq_by",   fn() ROWS.uniq_by("t"))
bench("Aggregate", "max_by",    fn() ROWS.max_by("n"))
bench("Aggregate", "min_by",    fn() ROWS.min_by("n"))
bench("Aggregate", "filter_by", fn() ROWS.filter_by("t", "type3"))
bench("Aggregate", "find_by",   fn() ROWS.find_by("n", N - 1))

