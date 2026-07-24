# {{APP_NAME}} — iOS shell

UIKit + WKWebView onto `{{START_URL}}`.

```bash
# needs XcodeGen: brew install xcodegen
xcodegen generate
open {{APP_NAME}}.xcodeproj
```

Free provisioning covers the custom scheme `{{SCHEME}}://` and most bridge
capabilities. Push, Universal Links, and NFC need a paid Apple Developer
account and the entitlements in `{{APP_NAME}}/App.entitlements`.

On APNs token receipt the shell POSTs to `/devices` using cookies from the
WebView (user must be logged in). Run `soli generate devices` on the server.
