# {{APP_NAME}} — Android shell

Thin WebView onto `{{START_URL}}`. Build without Gradle:

```bash
export ANDROID_HOME=…
./build.sh
```

Requires build-tools 35.0.0 and platform android-34.

Closed-app push needs Firebase — regenerate with:

```bash
soli generate client android --fcm --url {{START_URL}} --package {{PACKAGE_ID}}
```

Deep links: custom scheme `{{SCHEME}}://` and https `{{HOST}}` (pair with
`soli generate app_links` on the server).

Add launcher icons under `res/mipmap-*/ic_launcher.png` if you want a custom icon
(manifest ships without `android:icon` so aapt2 works with no assets).
