# VAPID / Web Push Functions

Soli ships native Web Push support so applications no longer need the `web-push` Node module. The four
builtins below cover the full RFC 8291 / 8292 stack:

| Function | What it does |
|---|---|
| `vapid_generate_keys()` | Generates a fresh P-256 application server key pair (base64url). |
| `vapid_sign(...)` | Signs the ES256 JWT that goes into `Authorization: vapid t=..., k=...`. |
| `vapid_encrypt(...)` | Encrypts a payload per RFC 8291 (`aes128gcm`) with fresh ephemeral keys. |
| `vapid_send(...)` | Signs + encrypts + POSTs to the subscription endpoint in one call. |

The encryption step always generates a *fresh* ephemeral P-256 keypair internally &mdash; VAPID identity
keys are never reused for ECDH (per RFC 8291 §3.4).

---

## Subscription shape

Every function that talks to a push service takes a *subscription* hash matching what the browser exposes
via `PushSubscription.toJSON()`:

```soli
let subscription = {
  "endpoint": "https://fcm.googleapis.com/fcm/send/abc...",
  "keys": {
    "p256dh": "BC4Q...",   # 65-byte uncompressed P-256 point, base64url
    "auth":   "qZQ..."     # 16-byte auth secret, base64url
  }
}
```

Persist this hash exactly as the browser sent it (the `subscribe()` response on the client). The push
service rejects sends if the `p256dh` or `auth` don't match the original subscription.

---

## vapid_generate_keys()

Generate a fresh P-256 ECDH/ECDSA key pair to identify this application server to push services
(RFC 8292 §3). Run this **once**, store both values in `.env`, and reuse them across deploys.
Subscriptions are bound to the public key &mdash; rotating it invalidates every existing subscription, so
you only do it when a key is compromised.

**Returns:** `Hash` &mdash; `{"public_key": String, "private_key": String}`. The public key is the 65-byte
uncompressed point; the private key is the 32-byte scalar. Both are base64url-encoded without padding.

**Example:**

```soli
let keys = vapid_generate_keys()
println("VAPID_PUBLIC_KEY=" + keys["public_key"])
println("VAPID_PRIVATE_KEY=" + keys["private_key"])
```

---

## vapid_sign(private_key, audience, subject, expiry_seconds?)

Sign an ES256 VAPID JWT for the `Authorization: vapid t=<jwt>, k=<public_key>` header
(RFC 8292 §2). `vapid_send` calls this internally &mdash; reach for `vapid_sign` directly only when
you're driving the push service request yourself.

**Parameters:**

| Name | Type | Description |
|---|---|---|
| `private_key` | `String` | The base64url private key from `vapid_generate_keys`. |
| `audience` | `String` | The `scheme://host[:port]` origin of the push endpoint, e.g. `https://fcm.googleapis.com`. Must be an absolute http(s) URL. |
| `subject` | `String` | A `mailto:` URI or HTTPS URL the push service can reach you at. |
| `expiry_seconds` | `Int?` | JWT lifetime, default 12 h; RFC 8292 caps at 24 h. |

**Returns:** `String` &mdash; the three-segment JWT (`header.payload.signature`).

**Example:**

```soli
let token = vapid_sign(
  getenv("VAPID_PRIVATE_KEY"),
  "https://fcm.googleapis.com",
  "mailto:ops@example.com"
)
# eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.eyJhdWQiOi...
```

---

## vapid_encrypt(payload, subscription, public_key, private_key)

Encrypt a Web Push payload per RFC 8291 (`aes128gcm`). The function generates a fresh ephemeral
P-256 keypair internally; the `public_key` / `private_key` parameters mirror `vapid_send`'s signature
for consistency but are not used by the encryption step.

**Parameters:**

| Name | Type | Description |
|---|---|---|
| `payload` | `String` | Cleartext body for the service worker (usually a JSON string). |
| `subscription` | `Hash` | Browser-issued `{endpoint, keys: {p256dh, auth}}`. |
| `public_key` | `String` | VAPID public key (kept for symmetry; unused). |
| `private_key` | `String` | VAPID private key (kept for symmetry; unused). |

**Returns:** `Hash` &mdash; `{"ciphertext": String, "salt": String, "server_public_key": String}`.
`ciphertext` is the full RFC 8291 record body (`salt || rs || idlen || ephemeral_pub || AES-GCM
ciphertext`) base64url-encoded; POST it verbatim with `Content-Encoding: aes128gcm`.

**Example:**

```soli
let result = vapid_encrypt(
  "{\"title\":\"Hello\"}",
  subscription,
  getenv("VAPID_PUBLIC_KEY"),
  getenv("VAPID_PRIVATE_KEY")
)
println(len(result["ciphertext"]))         # ~100+ chars
println(len(result["salt"]))               # 22 chars (16 bytes base64url no-pad)
println(len(result["server_public_key"]))  # 87 chars (65 bytes base64url)
```

---

## vapid_send(subscription, payload, private_key, public_key, subject, options?)

End-to-end Web Push delivery: signs the VAPID JWT, encrypts the payload, and POSTs the encrypted
record to the subscription's `endpoint`. This is the function most controllers call.

**Parameters:**

| Name | Type | Description |
|---|---|---|
| `subscription` | `Hash` | Browser-issued `{endpoint, keys: {p256dh, auth}}`. |
| `payload` | `String` | Body for the service worker (typically `json_stringify(...)`). |
| `private_key` | `String` | VAPID private key (base64url). |
| `public_key` | `String` | VAPID public key (base64url), sent as `k=` in Authorization. |
| `subject` | `String` | `mailto:` / https contact the push service can reach. |
| `options` | `Hash?` | Optional delivery hints (see below). |

**Options:**

| Key | Type | Description |
|---|---|---|
| `ttl` | `Int` | Push service `TTL` header in seconds (default 60). |
| `urgency` | `String` | `very-low`, `low`, `normal`, or `high` (RFC 8030). |
| `topic` | `String` | Replaces a queued message with the same topic on the same subscription. |
| `expiry_seconds` | `Int` | VAPID JWT lifetime (default 12 h). |

**Returns:** `Hash` &mdash; `{"status": Int, "body": String}`. A successful delivery is HTTP 201;
404/410 means the subscription is gone &mdash; delete it from your store.

**Example:**

```soli
let result = vapid_send(
  subscription,
  "{\"title\":\"New message\",\"body\":\"From Alice\"}",
  getenv("VAPID_PRIVATE_KEY"),
  getenv("VAPID_PUBLIC_KEY"),
  "mailto:ops@example.com",
  { "ttl": 3600, "urgency": "normal" }
)
if result["status"] == 410
  # Subscription is gone — clean it up.
  PushSubscription.delete(subscription["id"])
end
```

---

## End-to-end: provision keys, then send

**1. Generate keys once and store them in `.env`:**

```soli
# bin/vapid_keys.sl
let keys = vapid_generate_keys()
println("VAPID_PUBLIC_KEY=" + keys["public_key"])
println("VAPID_PRIVATE_KEY=" + keys["private_key"])
```

```bash
soli run bin/vapid_keys.sl >> .env
```

**2. Expose the public key to the browser** so the service worker can call
`registration.pushManager.subscribe({ applicationServerKey })` with it.

**3. Send from a controller action:**

```soli
# app/controllers/notifications_controller.sl
fn notify
  let subscription = PushSubscription.find(req["json"]["sub_id"])
  let result = vapid_send(
    {
      "endpoint": subscription["endpoint"],
      "keys": { "p256dh": subscription["p256dh"], "auth": subscription["auth"] }
    },
    json_stringify({ "title": "Hi", "body": req["json"]["text"] }),
    getenv("VAPID_PRIVATE_KEY"),
    getenv("VAPID_PUBLIC_KEY"),
    "mailto:ops@example.com"
  )
  return { "status": result["status"] }
end
```
