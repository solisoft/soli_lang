# Release Skill

Complete development workflow: run lint (clippy + fmt), tests, commit changes, and create a release.

## Workflow

1. **Run clippy** with warnings as errors
2. **Run cargo fmt** to ensure consistent formatting
3. **Run tests** (`cargo test --lib`)
4. **Commit** all changes with an automated message
5. **Push** to remote
6. **Create release** via `./scripts/release.sh` (default: minor bump)

## Usage

```
/release                    # Run full workflow with minor release
/release patch             # Patch release
/release major             # Major release
/release --dry-run         # Preview without making changes
```

## Commands Executed

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test --lib
git add -A
git commit -m "chore: release v[new-version]"
git push
./scripts/release.sh <type>  # default: minor
```

## Requirements

- Clean working tree (no uncommitted changes before starting)
- Must be on `main` branch
- Remote must be configured for push