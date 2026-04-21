---
name: release
description: Complete development workflow: run lint (clippy + fmt), tests, update changelog, commit changes, and create a release.
---
## Workflow

1. **Run clippy** with warnings as errors
2. **Run cargo fmt** to ensure consistent formatting
3. **Run Rust tests** (`cargo test --lib`)
4. **Run Soli tests** (`soli test tests/`)
5. **Update CHANGELOG.md** with unreleased changes
6. **Commit** all changes with an automated message
7. **Push** to remote
8. **Create release** via `./scripts/release.sh` (default: minor bump)

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
soli test tests/
# Update CHANGELOG.md with unreleased changes
git add -A
git commit -m "chore: release v[new-version]"
git push
./scripts/release.sh <type>  # default: minor
```

## Updating CHANGELOG.md

Before committing, update `CHANGELOG.md`:

1. Extract changes from git diff (features, bug fixes, etc.)
2. Add entries under `[Unreleased]` section in appropriate categories:
   - `### Features` - new functionality
   - `### Bug Fixes` - bug corrections
   - `### Documentation` - docs updates
   - `### Refactoring` - code restructuring
3. If no unreleased changes exist, skip this step

Example format:
```markdown
## [Unreleased]

### Features

* **feature-name:** description ([commit-hash](link))

### Bug Fixes

* **component:** fix description ([commit-hash](link))
```

## Requirements

- Clean working tree (no uncommitted changes before starting)
- Must be on `main` branch
- Remote must be configured for push