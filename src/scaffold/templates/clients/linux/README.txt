# {{APP_NAME}} — Linux shell

GTK 3 + WebKitGTK WebView onto `{{START_URL}}`.

```bash
# Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev
cargo build --release
./target/release/{{SCHEME}}-shell
# optional system install:
./install.sh
```

Deep links: `{{SCHEME}}://path` is registered via the `.desktop` file
`MimeType=x-scheme-handler/{{SCHEME}}`.
