# Web Push from Soli: Goodbye `web-push`, Hello Native VAPID

Web Push notifications are one of those features every product eventually wants and few developers actually enjoy wiring up. The protocol stack is small but unforgiving: ECDH on a fresh ephemeral key per message, HKDF, AES-128-GCM, an ES256 JWT pinned to the push service's origin — and then a single misplaced base64url pad will make Chrome's push service silently drop your messages.

For years, the answer in the Node ecosystem has been the `web-push` package. It's good. It is also a couple of megabytes of transitive dependencies, an extra runtime to keep alive next to your app, and one more thing to audit when a CVE drops.

Soli ships the whole stack natively. Four builtins — `vapid_generate_keys`, `vapid_sign`, `vapid_encrypt`, `vapid_send` — cover RFC 8291 and RFC 8292 end to end. No `npm install`, no sidecar, no FFI. This post walks you through dropping `web-push` and sending your first push from a Soli controller.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/web-push-vapid.jpg" width="1024" height="576" alt="Web Push notification flow with native Soli VAPID: Soli server generates keys and sends encrypted push via VAPID, through browser push service, directly to the user's device with no external npm dependencies." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Native VAPID in Soli: the entire push pipeline lives inside your single binary.</figcaption>
</figure>

## Before / After

If you have ever delivered Web Push from a Node service, this is roughly the shape of it:

```js
// Node + web-push
const webpush = require("web-push");

webpush.setVapidDetails(
  "mailto:ops@example.com",
  process.env.VAPID_PUBLIC_KEY,
  process.env.VAPID_PRIVATE_KEY
);

await webpush.sendNotification(
  subscription,
  JSON.stringify({ title: "Hi", body: "From Alice" }),
  { TTL: 3600 }
);
```

The Soli equivalent is one builtin call:

```soli
# Soli, native
vapid_send(
  subscription,
  json_stringify({"title": "Hi", "body": "From Alice"}),
  getenv("VAPID_PRIVATE_KEY"),
  getenv("VAPID_PUBLIC_KEY"),
  "mailto:ops@example.com",
  {"ttl": 3600}
)
```

Same RFCs, same wire bytes, none of the npm graph. The rest of this post fills in the four steps around that one line: generating keys, exposing them to the browser, persisting subscriptions, and sending.

## How Web Push Actually Works

Before we touch code, the moving parts:

1. **Your server holds a long-lived VAPID key pair.** The public key identifies your application to every push service (FCM, Mozilla, Apple). The private key signs a short-lived JWT on every send.
2. **The browser asks for permission**, then calls `pushManager.subscribe({ applicationServerKey })`. The push service hands back an opaque endpoint URL and two short keys — `p256dh` (the user agent's public ECDH key) and `auth` (a 16-byte secret).
3. **Your server stores that subscription.** It is per-browser-per-device. If the user reinstalls, you get a new one.
4. **To send a push**, you encrypt the payload with the user's `p256dh` and `auth` using a fresh ephemeral key (RFC 8291), sign a VAPID JWT (RFC 8292), and POST the encrypted body to the subscription's `endpoint`.

Soli's builtins handle steps 3's encryption and step 4's signing/POSTing for you.

## Step 1: Generate VAPID Keys Once

VAPID keys are long-lived. You generate them once, store both halves in `.env`, and reuse them for the life of the app. Rotating the public key invalidates every existing subscription, so this is something you do exactly once per deploy and only again when the private key is compromised.

A throwaway script will do:

```soli
# bin/vapid_keys.sl
let keys = vapid_generate_keys()
println("VAPID_PUBLIC_KEY=" + keys["public_key"])
println("VAPID_PRIVATE_KEY=" + keys["private_key"])
```

Run it once and append the output to `.env`:

```bash
soli run bin/vapid_keys.sl >> .env
```

The returned strings are base64url with no padding. `public_key` is the 65-byte uncompressed P-256 point you will hand to the browser; `private_key` is the 32-byte scalar that signs the JWT.

## Step 2: Expose the Public Key to the Browser

The browser needs the public key in raw byte form to call `pushManager.subscribe`. Expose it via a small JSON endpoint:

```soli
# app/controllers/push_controller.sl

def vapid_public_key
  {"status": 200, "json": {"public_key": getenv("VAPID_PUBLIC_KEY")}}
end
```

```soli
# config/routes.sl
get("/push/public-key", "push#vapid_public_key")
post("/push/subscribe",  "push#subscribe")
```

On the client, the service worker registers and subscribes:

```js
// public/js/push.js
async function enablePush() {
  const reg = await navigator.serviceWorker.register("/sw.js");
  const { public_key } = await fetch("/push/public-key").then(r => r.json());

  const subscription = await reg.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: urlBase64ToUint8Array(public_key),
  });

  await fetch("/push/subscribe", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(subscription.toJSON()),
  });
}

function urlBase64ToUint8Array(b64) {
  const pad = "=".repeat((4 - b64.length % 4) % 4);
  const s = (b64 + pad).replace(/-/g, "+").replace(/_/g, "/");
  const raw = atob(s);
  return Uint8Array.from(raw, c => c.charCodeAt(0));
}
```

`subscription.toJSON()` produces the exact shape Soli's builtins expect:

```json
{
  "endpoint": "https://fcm.googleapis.com/fcm/send/abc...",
  "keys": { "p256dh": "BC4Q...", "auth": "qZQ..." }
}
```

Store that hash byte-for-byte — the push service rejects sends if `p256dh` or `auth` don't match the original subscription.

## Step 3: Persist Subscriptions

A subscription belongs to a user (or a device, if you do anonymous push). A minimal model:

```soli
# app/models/push_subscription.sl
class PushSubscription < Model
  # Inherits all(), find(id), where(...), create(...), etc.
end
```

```soli
# app/controllers/push_controller.sl
def subscribe
  let json = req["json"]

  PushSubscription.create({
    "user_id":  req["session"]["user_id"],
    "endpoint": json["endpoint"],
    "p256dh":   json["keys"]["p256dh"],
    "auth":     json["keys"]["auth"]
  })

  {"status": 201, "body": ""}
end
```

A user with multiple devices ends up with multiple rows — fan out the send across all of them when you want every device to ring.

## Step 4: Send a Push

This is the line that replaces `web-push`. Look up the subscription, hand it to `vapid_send`, done:

```soli
# app/controllers/notifications_controller.sl

def notify
  let sub = PushSubscription.find(req["json"]["sub_id"])

  let result = vapid_send(
    {
      "endpoint": sub["endpoint"],
      "keys": {"p256dh": sub["p256dh"], "auth": sub["auth"]}
    },
    json_stringify({"title": "Hi", "body": req["json"]["text"]}),
    getenv("VAPID_PRIVATE_KEY"),
    getenv("VAPID_PUBLIC_KEY"),
    "mailto:ops@example.com",
    {"ttl": 3600, "urgency": "normal"}
  )

  if result["status"] == 410 or result["status"] == 404
    # The browser has uninstalled or revoked — clean it up.
    PushSubscription.delete(sub["id"])
  end

  {"status": result["status"]}
end
```

`vapid_send` signs the ES256 JWT, generates a fresh ephemeral P-256 keypair for ECDH, encrypts the payload with `aes128gcm` per RFC 8291, and POSTs the record to `sub["endpoint"]`. The `options` hash is where you set the push service's `TTL`, `urgency` (`very-low`, `low`, `normal`, `high`), and `topic` (which replaces a queued message with the same topic on the same subscription).

A `201` means delivered to the push service. `404` and `410` mean the subscription is dead — always delete those, or your cleanup job will get bigger every week.

## Fanning Out to Every Device

Push from a controller usually means "ring all of this user's devices":

```soli
def broadcast(user_id, payload)
  let subs = PushSubscription.where({"user_id": user_id})
  let body = json_stringify(payload)
  let dead = []

  for sub in subs
    let r = vapid_send(
      {"endpoint": sub["endpoint"],
       "keys": {"p256dh": sub["p256dh"], "auth": sub["auth"]}},
      body,
      getenv("VAPID_PRIVATE_KEY"),
      getenv("VAPID_PUBLIC_KEY"),
      "mailto:ops@example.com"
    )
    dead.push(sub["id"]) if r["status"] == 410 or r["status"] == 404
  end

  for id in dead
    PushSubscription.delete(id)
  end
end
```

In a real app, push this onto a background job rather than blocking the request — push services occasionally take a few hundred milliseconds, and the user clicking "send" doesn't need to wait for fan-out.

## When You Want the Lower-Level Builtins

`vapid_send` is the one most controllers reach for, but the other three are there when you need them:

- `vapid_sign(private_key, audience, subject, expiry_seconds?)` returns just the ES256 JWT, useful if you proxy through an internal gateway that needs the `Authorization: vapid t=..., k=...` header but does the POST itself.
- `vapid_encrypt(payload, subscription, public_key, private_key)` returns the encrypted record body (`ciphertext`, `salt`, `server_public_key`) so you can stash it somewhere and replay later.
- `vapid_generate_keys()` is the one-time setup we used above.

All four are documented in detail at [/docs/builtins/vapid](/docs/builtins/vapid).

## Why This Matters

The point of bundling VAPID into Soli is not "we wrote a clone of `web-push`". It is that Web Push is now one of the dozen-or-so things you do not have to import, audit, version-pin, or polyglot-wrangle to ship. The protocol is small enough that it belongs in the framework, the way HTTP and JSON do. One language, one runtime, one `.env`, and the user gets a notification.

That is the trade Soli is built around.
