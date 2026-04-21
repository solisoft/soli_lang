# /doc Skill

Update documentation about current changes in the www/ folder (.md and .slv files).

## Usage

```
/doc
/doc list
/doc read <file>
```

## Workflow

1. **List documentation files**: Run `/doc` or `/doc list` to see all .md and .slv files in www/
2. **Read a file**: Run `/doc read <path>` to read a specific file
3. **Update documentation**: When you make changes to the codebase that affect documented features, find the corresponding doc file(s) and update them

## Documentation File Locations

- `.md` files: `www/docs/` - these are source documentation files
- `.slv` files: `www/app/views/docs/` - these are Soli view templates rendered by the web app

## Search Index

When adding new builtin functions or language features, also update the search index:
- `www/public/js/search-index.json` - Searchable entries for the documentation site

The search index entries follow this format:
```json
{
  "title": "function_name",
  "type": "function",
  "category": "Core",
  "path": "/docs/builtins/core#fn-function-name",
  "signature": "function_name(param)",
  "description": "Brief description",
  "keywords": ["related", "keywords"]
}
```

## Common Tasks

- If you add a new builtin function, update `www/docs/builtins.md`, `www/app/views/docs/builtins/`, AND `www/public/js/search-index.json`
- If you add a new language feature, update `www/docs/soli-language.md` or the relevant language doc
- If you add a new core concept, create both `.md` and `.slv` documentation
- After updating docs, verify search index entries exist for new content

## Commands

- `list` - List all documentation files in www/ folder
- `read <path>` - Read a specific documentation file
- `update` - Update the search-index.json when adding new functions/features