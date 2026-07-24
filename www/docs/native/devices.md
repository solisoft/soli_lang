# Device registration

Store push targets so [`Push.deliver`](/docs/native/notifications) can reach a
user on every platform.

The framework owns **routing** and **dead-token detection**. Your app owns the
**device list** — where tokens live is schema-dependent.
`soli generate devices` scaffolds the usual shape.

## Generate

```bash
soli generate devices
soli db:migrate up
```

That adds:

| Piece | Role |
|-------|------|
| `app/models/device.sl` | `platform`, `token`, optional web `subscription`; upsert by token |
| `POST /devices` | Register after login (session required) |
| `DELETE /devices/:id` | Unregister |
| `Device.push_targets_for(user_id)` | Array for `Push.deliver` options |
| `Device.prune_tokens(tokens)` | Delete rows the push service reported gone |
| `deliver_to_user(user_id, payload, options)` | Deliver + prune in one helper |
| `skip_csrf("/devices/*")` | Shell token POSTs often lack Origin (see below) |

Migrations use `begin`/`rescue` so a re-run or an auto-created collection is not
a hard failure.

## Layout

```erb
<%- csrf_meta_tag() %>
<% user_id = session_get("user_id") %>
<%- native_channel("user:#{str(user_id)}") rescue "" unless user_id.nil? %>
```

- `csrf_meta_tag` — so page-side `registerDevice` can send `X-CSRF-Token`
- `native_channel` — open-app bridge notifications over SSE

Requires `SOLI_SESSION_SECRET` (32+ characters).

## HTTP contract

```http
POST /devices
Cookie: <session>
Content-Type: application/json
# from a page (recommended when meta is present):
X-CSRF-Token: <csrf_meta_tag content>

{ "platform": "android", "token": "…" }
```

| Field | |
|-------|--|
| `platform` | `ios`, `android`, `web`, `macos`, `apple`, or `fcm` |
| `token` | Device token (required unless web uses `subscription` only) |
| `subscription` | Web Push subscription object (`platform: "web"`) |

```http
→ 201 { "id": "…", "platform": "android", "token": "…" }
→ 401 login required
→ 422 invalid platform / missing token
```

### CSRF

Native shells often POST with a **session cookie but no Origin** (plain
`HttpURLConnection` / `URLSession`). The generator therefore adds:

```soli
skip_csrf("/devices/*");
```

The controller still requires a logged-in session. Generated iOS and Android-FCM
clients also set `Origin` / `Referer` to your app origin.

From a page inside the WebView (same origin), prefer the bridge helper:

```js
// After login — Web Push:
await soli.nativeBridge.registerDevice({
  platform: "web",
  subscription: pushSubscriptionJson
})

// Or a shell-exposed token:
await soli.nativeBridge.registerDevice({
  platform: "android",
  token: fcmToken
})
```

`registerDevice` POSTs `/devices` with `credentials: "same-origin"` and copies
`X-CSRF-Token` from `<meta name="csrf-token">` when present.

## Sending

```soli
result = deliver_to_user(user.id, {
  "title": "New ping",
  "body":  "Ana replied",
  "url":   "/pings/3",
  "badge": 3
}, {
  "apns": { "key": apns_key, "key_id": "…", "team_id": "…", "topic": "net.example.app" },
  "fcm":  { "service_account": firebase_account }
  # VAPID from VAPID_* env when not passed
})
```

Or manually:

```soli
result = Push.deliver("user:#{str(user.id)}", payload, {
  "targets": Device.push_targets_for(user.id),
  "apns": apns_options,
  "fcm": fcm_options
})
Device.prune_tokens(result["prune"] || [])
```

**Always act on `prune`.** Tokens reported `410` / `UNREGISTERED` are gone for
good; a store that never deletes them grows without bound.

Open-app path only (no closed-app fallthrough):

```soli
Native.notify("user:#{str(user.id)}", payload)
```

## Who posts the token?

| Client | How the token reaches `/devices` |
|--------|----------------------------------|
| Generated iOS shell | APNs registration → POST with WebView cookies |
| Generated Android `--fcm` | FCM token → POST with cookies + Origin |
| Browser / PWA | Your page: Web Push subscribe → `registerDevice` |
| Custom shell | Same contract: session cookie + JSON body |

## Checklist

1. `SOLI_SESSION_SECRET` set  
2. `soli generate devices` + migrate  
3. Layout: `csrf_meta_tag` + `native_channel`  
4. Shell or page registers tokens after login  
5. `deliver_to_user` (or `Push.deliver` + prune) with APNs/FCM/VAPID credentials  
6. [`soli generate client`](/docs/native/clients) pointed at the deployment  

## Related

- [Notifications](/docs/native/notifications)
- [Native clients](/docs/native/clients)
- [Native Bridge](/docs/development-tools/native-bridge)
- [Android Push (FCM)](/docs/native/push-android)
- [Apple Push (APNs)](/docs/native/push-apple)
- [Native mobile blog](/docs/blog/native-mobile)
