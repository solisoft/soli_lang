# {{APP_NAME}} — Android shell (FCM)

Gradle project with Firebase Cloud Messaging for **closed-app** push.

1. Create a Firebase project and download `app/google-services.json`.
2. Place it at `app/google-services.json`.
3. Build:

```bash
./gradlew :app:assembleRelease
```

On token refresh the shell POSTs to `{{START_URL}}devices` with the session
cookie from the WebView (user must be logged in). Pair with:

```bash
soli generate devices
```

and `Push.deliver` / `Fcm.send` on the server.
