# mail-builder 0.4.4, made to run in a Cloudflare Worker

Why: Soli's mailer builds every message with `mail-builder`, and the edge
build (wasm32-unknown-unknown, Cloudflare Workers) could not: the crate reads
the clock through `std::time::SystemTime` — for the `Date` header, the
`Message-ID` and every multipart boundary — and `std::time` panics on that
target ("time not implemented on this platform"), taking the isolate with it.

What: the published `mail-builder-0.4.4.crate` (sha256
`900998f307338c4013a28ab14d760b784067324b164448c6d98a89e44810473b`, the
checksum Cargo.lock recorded) with three changes:

- `src/mime.rs`, `src/headers/date.rs`: `web_time::{SystemTime, …}` instead of
  `std::time` — the same API, backed by `Date.now()` on wasm and by
  `std::time` everywhere else.
- `src/lib.rs`: the `gethostname` Message-ID branch is native-only; a Worker
  has no host name, so it falls back to "localhost", as the crate does without
  the feature.
- `Cargo.toml`: `web-time` added; `gethostname` made a native-only dependency;
  the `[[example]]` and `[dev-dependencies]` sections dropped (their files are
  not vendored).

Wired in by `[patch.crates-io]` in the root Cargo.toml. Drop it if upstream
takes a `web-time` (or `wasm`) feature.
