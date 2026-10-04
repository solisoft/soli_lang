# {{APP_NAME}} — Android shell (FCM)

Gradle project with Firebase Cloud Messaging for **closed-app** push.

1. Create a Firebase project and download `app/google-services.json`.
2. Place it at `app/google-services.json`.
3. Build, from the app's folder:

```bash
soli mobile build android --fcm    # debug-signed, versioned APK in dist/mobile/
```

`assembleRelease` alone produces an unsigned APK that Android will not
install; `soli mobile build android --fcm --keystore release.jks` signs it
(passwords from `SOLI_ANDROID_KEYSTORE_PASSWORD` / `SOLI_ANDROID_KEY_ALIAS`).
No Gradle wrapper is generated: install Gradle, or run `gradle wrapper` here.

On token refresh the shell POSTs to `{{START_URL}}devices` with the session
cookie from the WebView (user must be logged in). Pair with:

```bash
soli generate devices
```

and `Push.deliver` / `Fcm.send` on the server.
