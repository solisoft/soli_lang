# Apple Push (APNs)

Reach a macOS or iOS app that is **closed**.

[Notifications](/docs/native/notifications) stop at the edge of "app is open" — a closed app is not
executing, so something else has to be listening. On Apple platforms that is APNs: the OS receives
the push and displays it whether or not your app is running.

## Sending

```soli
Apns.send(device_token, {
  "title": "New ping",
  "body":  "Ana replied to your comment",
  "url":   "/pings/3"
}, {
  "key":     File.read("AuthKey_ABC123.p8"),
  "key_id":  "ABC123DEFG",
  "team_id": "1A2B3C4D5E",
  "topic":   "net.example.myapp"
})
# => { "status": 200, "reason": "" }
```

## Handling the result

It returns rather than raises, because a dead device token is an ordinary outcome:

```soli
class ApplePush
  static def deliver(device, payload)
    result = Apns.send(device["token"], payload, ApplePush.credentials())

    # The device is gone: stop sending to it, or you will do this forever.
    if result["status"] == 410 || result["reason"] == "Unregistered"
      Device.find(device["_key"]).destroy()
      return "pruned"
    end

    return "sent" if result["status"] == 200

    Rails.logger("apns #{result["status"]} #{result["reason"]}")
    "failed"
  end
end
```

| Status | Reason | Meaning |
|---|---|---|
| `200` | — | Accepted by Apple. |
| `400` | `BadDeviceToken` | Almost always the wrong gateway — a development build's token sent to production. Add `"sandbox": true`. |
| `403` | `InvalidProviderToken` | The `.p8`, `key_id` or `team_id` do not agree. |
| `410` | `Unregistered` | App removed. Delete the token. |
| `429` | `TooManyProviderTokenUpdates` | Minting too often — see below. |

## Options

| Option | |
|---|---|
| `key` | Contents of the `.p8` file, BEGIN/END lines included. **Required.** |
| `key_id` | The key's 10-character id. **Required.** |
| `team_id` | Your Apple team id. **Required.** |
| `topic` | The app's bundle id. **Required.** |
| `sandbox` | `true` for development builds. Default `false`. |
| `priority` | `10` immediate (default) or `5` power-considerate. |
| `push_type` | `"alert"` (default), `"background"`, `"voip"`, … |
| `collapse_id` | Notifications sharing one replace each other. |
| `expiration` | Unix time after which Apple stops trying. |

`title`, `body`, `badge` and `sound` are wrapped into the `aps` envelope for you. Anything else rides
along as custom data the app reads on tap:

```soli
Apns.send(token, {
  "title":      "Deploy finished",
  "body":       "v1.24.0 is live",
  "badge":      1,
  "deploy_id":  "d_8123",          # custom — your app reads this
  "url":        "/deploys/d_8123"
}, options)
```

Supply your own `aps` key and it is sent verbatim, for anything this shape does not cover.

## Token-based auth

One `.p8` key works for every app under a team and never expires, where certificates are per-app and
expire annually. Get one from *Apple Developer → Keys*, enable APNs on it, and note the key id.

Provider tokens are cached for 45 minutes, and that is not an optimization: **Apple rate-limits
minting**, answering `TooManyProviderTokenUpdates` if you reissue more often than every 20 minutes.
`Apns.token(key, key_id, team_id)` exposes one for a caller driving the HTTP itself.

## What it costs

Unlike the bridge, this needs setup — and one prerequisite has no way around it:

**Receiving requires the `aps-environment` entitlement**, which comes from a provisioning profile,
which requires a **paid Apple Developer account**. An ad-hoc signed app cannot receive a push however
correct the sender is.

Your app then registers and reports its token:

```swift
NSApplication.shared.registerForRemoteNotifications()

func application(_ app: NSApplication,
                 didRegisterForRemoteNotificationsWithDeviceToken token: Data) {
    let hex = token.map { String(format: "%02x", $0) }.joined()
    // POST it to your app and store it against the signed-in user.
}
```

## Combining the two

```soli
reached = Native.notify("user:#{str(user_id)}", payload)
Apns.send(device_token, payload, apns_options) if reached == 0
```

The bridge for anyone looking, APNs for anyone who is not — and no push service involved in the
common case.
