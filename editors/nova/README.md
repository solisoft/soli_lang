# Nova editor support for Soli

The actual Nova extension bundle lives under
[`soli.novaextension/`](./soli.novaextension). That folder is what Nova loads
— see its README for installation and feature notes.

## Layout

```
editors/nova/
└── soli.novaextension/
    ├── extension.json        # Manifest (id, version, contributes)
    ├── README.md             # User-facing docs (shown in the Extension Library)
    ├── CHANGELOG.md
    ├── Scripts/
    │   └── main.js           # activate() spawns `soli lsp` via LanguageClient
    └── Syntaxes/
        └── soli.tmLanguage.json   # TextMate grammar (shared with VS Code)
```

## Releasing

Nova extensions are published through the Nova Extension Library. From a Mac
with Nova installed:

```bash
# Open Nova → Extensions → Publish… and select this directory.
open -a Nova editors/nova/soli.novaextension
```

The LSP backend the extension talks to is `soli lsp`, which is defined in
`src/lsp/` and wired up in `src/cli/`. Bumping the LSP requires no change to
the extension as long as the LSP capabilities don't shrink.
