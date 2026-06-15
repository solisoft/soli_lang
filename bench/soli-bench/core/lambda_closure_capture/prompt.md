# lambda_closure_capture

Implement a function `make_counter(start)` that returns a closure. Each
call to the returned closure increments and returns the counter.

```soli
let c1 = make_counter(10);
c1();  // => 10
c1();  // => 11
c1();  // => 12

let c2 = make_counter(100);
c2();  // => 100   (independent of c1)
c1();  // => 13    (c1's state preserved)
```

**Idiomatic touches we want to see**
- A `fn` lambda (not the `|x| { ... }` pipe lambda — those are equivalent
  for the body, but `fn` is the canonical form for stored closures).
- The returned lambda captures `start` by reference / closure, not by value.
- The captured counter is mutated across calls (so the lambda is a true
  closure, not a snapshot).
