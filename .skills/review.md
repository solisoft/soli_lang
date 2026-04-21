# Code Review Skill

Review code changes and provide feedback on quality, style, and potential issues.

## Usage

```
/review                    # Review all changes
/review --files <paths>    # Review specific files
/review --lint             # Run linters only
/review --test             # Run tests only
```

## Review Checklist

### Code Quality
- [ ] No dead code or unreachable code
- [ ] No empty catch blocks
- [ ] Proper error handling
- [ ] No memory leaks (for Rust code)

### Style & Conventions
- [ ] Follows naming conventions (snake_case for functions/variables, PascalCase for classes)
- [ ] No overly long functions (>100 lines)
- [ ] Proper documentation for public APIs
- [ ] No commented-out code

### Testing
- [ ] New functionality has tests
- [ ] Tests pass
- [ ] Edge cases handled

### Security
- [ ] No hardcoded secrets
- [ ] Input validation
- [ ] Proper escaping (for web code)

## Commands Executed

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test --lib
soli lint
soli test tests/
git diff --stat
```

## Notes

- Review focuses on Soli Lang codebase
- Rust code uses clippy for linting
- Soli code uses `soli lint` for style issues