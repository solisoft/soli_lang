#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release
BIN="target/release/{{SCHEME}}-shell"
install -Dm755 "$BIN" "${HOME}/.local/bin/{{SCHEME}}-shell"
install -Dm644 "{{SCHEME}}.desktop" "${HOME}/.local/share/applications/{{SCHEME}}.desktop"
xdg-mime default "{{SCHEME}}.desktop" x-scheme-handler/{{SCHEME}}
echo "Installed {{SCHEME}}-shell and registered {{SCHEME}}://"
