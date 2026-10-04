# {{APP_NAME}} — Android shell

Thin WebView onto `{{START_URL}}`. Build without Gradle, from the app's folder:

```bash
export ANDROID_HOME=…
soli mobile build android      # versioned APK in dist/mobile/
soli mobile publish --latest android   # upload it to /__soli/mobile
```

or by hand here with `./build.sh` (writes `app.apk`, unversioned).
Requires build-tools 35.0.0 and platform android-34. Keep `debug.keystore`
once it exists: an APK signed with another key will not install as an update.

Closed-app push needs Firebase — regenerate with:

```bash
soli generate client android --fcm --url {{START_URL}} --package {{PACKAGE_ID}}
```

Deep links: custom scheme `{{SCHEME}}://` and https `{{HOST}}` (pair with
`soli generate app_links` on the server).

Add launcher icons under `res/mipmap-*/ic_launcher.png` if you want a custom icon
(manifest ships without `android:icon` so aapt2 works with no assets).
