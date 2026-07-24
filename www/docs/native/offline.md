# Offline mobile (v1)

Soli mobile shells are **WebViews onto a remote deployment**. That is the right
default for always-online products. True offline is a different product shape.

## What desktop already is

[`soli desktop build`](/docs/development-tools/desktop) freezes Soli + SolidB into
one local executable. That is the offline engine for field tools that must not
depend on a network after unlock (aside from the launch key — see desktop docs).

Mobile does **not** embed a general-purpose server (iOS policy; battery and
update story). Do not expect desktop parity on the phone.

## Generate the v1 scaffold

```bash
soli generate offline
soli db:migrate up
```

That adds:

| Piece | Role |
|-------|------|
| `POST /sync/push` | Accept outbox items |
| `GET /sync/pull?since=` | Pull recent `SyncEvent` rows for the signed-in user |
| `app/models/sync_event.sl` | Optional audit log of drained items |
| `public/js/soli_outbox.js` | Browser/shell outbox in `localStorage` + flush on `online` |
| `skip_csrf("/sync/*")` | Shell flushes may lack Origin; session still required |

### Outbox payload

```json
{
  "items": [
    {
      "id": "client-uuid",
      "method": "POST",
      "path": "/pings",
      "body": { "text": "hi" },
      "created_at": "2026-07-24T12:00:00Z"
    }
  ]
}
```

```html
<script src="/js/soli_outbox.js"></script>
<script>
  soliOutbox.enqueue({ method: "POST", path: "/pings", body: { text: "hi" } })
  // later or automatically on window "online":
  await soliOutbox.flush()
</script>
```

Default `sync#push` **stores** events for inspection. Replace the loop body with
real model creates/updates for your domain. The server remains the source of
truth; the client only queues until flush succeeds.

### Auth

Both endpoints require `session_get("user_id")`. Unauthenticated flushes get
`401`. Pair with your login flow the same way as
[device registration](/docs/native/devices).

## PWA limits

iOS home-screen PWAs got better (push 16.4+), but background sync and storage
quotas still differ from Android. Prefer the native shell when offline write
queues are product-critical.

## What is deliberately out of v1

- Full embedded SolidB on iOS/Android  
- Automatic conflict CRDTs  
- Background location while offline — see [Platform limits](/docs/native/platform-limits)

## Related

- [Desktop Applications](/docs/development-tools/desktop)
- [Native clients](/docs/native/clients)
- [Device registration](/docs/native/devices)
- [Native Bridge](/docs/development-tools/native-bridge)
