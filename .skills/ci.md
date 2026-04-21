# CI Skill

Run the full CI pipeline: clippy, fmt, and tests.

## Usage

```
/ci
```

## Commands Executed

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test --lib
```

## Notes

- Does not commit or push
- Does not create a release
- Use `/release` to complete the full workflow including release