# Android Push (FCM)

Reach an Android app that is **closed**.

Android kills long-lived connections within minutes of the screen going off. A background socket or
a foreground service is not a workaround — it is a battery cost that still loses to Doze. FCM exists
for exactly this.

## Sending

```soli
Fcm.send(device_token, {
  "title": "New ping",
  "body":  "Ana replied to your comment",
  "url":   "/pings/3"
}, {
  "service_account": File.read("service-account.json")
})
# => { "status": 200, "reason": "" }
```

## Handling the result

```soli
class AndroidPush
  static def deliver(device, payload)
    result = Fcm.send(device["token"], payload, { "service_account": AndroidPush.account() })

    # The app was uninstalled or the token rotated.
    if result["reason"] == "UNREGISTERED" || result["reason"] == "NOT_FOUND"
      Device.find(device["_key"]).destroy()
      return "pruned"
    end

    result["status"] == 200 ? "sent" : "failed"
  end
end
```

| Status | Reason | Meaning |
|---|---|---|
| `200` | — | Accepted by Google. |
| `400` | `INVALID_ARGUMENT` | Malformed token or payload. |
| `404` | `UNREGISTERED` | App removed or token rotated. Delete it. |
| `429` | `QUOTA_EXCEEDED` | Back off and retry. |
| `503` | `UNAVAILABLE` | Transient. Retry with backoff. |

## Payload mapping

`title` and `body` become a `notification`, which is what makes Android display the message with the
app closed. Everything else becomes `data` for the app to read on tap:

```soli
Fcm.send(token, {
  "title":   "Build finished",
  "body":    "main is green",
  "build":   4821,             # becomes data, stringified to "4821"
  "url":     "/builds/4821"
}, options)
```

**`data` values are stringified**, deliberately: FCM rejects a `data` payload containing numbers or
booleans *outright*, so an innocuous `{"count": 3}` would otherwise fail the entire send. Every FCM
client library does the same.

Messages go out at `high` priority, because normal priority is precisely what Doze defers — which
would defeat the purpose of reaching a sleeping device.

Supply your own `message` key and it is sent verbatim, for topics, conditions or per-platform
overrides:

```soli
Fcm.send("", { "message": {
  "topic": "release-notes",
  "notification": { "title": "v1.24.0", "body": "See what changed" }
}}, options)
```

## Why this needs more setup than APNs

APNs authenticates with the request itself. FCM's HTTP v1 API wants an **OAuth2 access token**, so
there is an extra round trip: sign a service-account assertion (RS256), exchange it at Google's token
endpoint, then send. Access tokens last an hour and are cached per service account, so the exchange
happens roughly once rather than per message.

The old `key=AAAA…` server-key API needed none of that. It is not an option — Google shut it down in
2024.

`Fcm.access_token(service_account_json)` exposes the token for a caller driving the HTTP itself.

## What it costs

1. A Firebase project (free).
2. A service-account JSON from *Project settings → Service accounts*. Treat it as a secret: it can
   send as your app to anyone.
3. **The Firebase SDK in the Android app** — this is the real work. Obtaining a device token requires
   `firebase-messaging`, which needs `google-services.json` and a **Gradle build**.

That last point is a build-system change, not just code: a shell built with `aapt2 → javac → d8` and
no dependency resolution cannot absorb the Play Services dependency tree.

Once it is in, the app reports its token the same way:

```java
FirebaseMessaging.getInstance().getToken()
    .addOnSuccessListener(token -> {
        // POST it to your app and store it against the signed-in user.
    });
```

## Combining the two

```soli
reached = Native.notify("user:#{str(user_id)}", payload)
Fcm.send(device_token, payload, fcm_options) if reached == 0
```
