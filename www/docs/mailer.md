# Mailer

Soli ships a first-class mailer for sending transactional email over SMTP. You
define a mailer the way you define a controller — a class with one method per
email — set instance variables, render a view, and send synchronously or in the
background.

## Defining a mailer

Mailers live in `app/mailers/` and subclass `Mailer`. Each public method is an
*action* that builds and returns a message with `this.mail(...)`:

```soli
# app/mailers/user_mailer.sl
class UserMailer < Mailer
  def welcome(user)
    @user = user
    # Renders app/views/user_mailer/welcome.html.slv with @user in scope.
    this.mail(to: user.email, subject: "Welcome!")
  end

  def reset_password(user)
    @user = user
    this.mail(to: user.email, subject: "Reset your password")
  end
end
```

Generate one with:

```bash
soli generate mailer User welcome reset_password
```

This writes `app/mailers/user_mailer.sl` plus an HTML view per action under
`app/views/user_mailer/`.

## Views

A mailer view is a normal `.slv` template. The action's instance variables are
available as locals, exactly like a controller view:

```erb
<!-- app/views/user_mailer/welcome.html.slv -->
<h1>Welcome, <%= h(user.name) %>!</h1>
<p>Thanks for joining. <a href="https://example.com/start">Get started</a>.</p>
```

The view is chosen by convention from the class and action names —
`UserMailer#welcome` renders `user_mailer/welcome`. Override it with
`template:`, or skip rendering entirely by passing `html:` (and/or `text:`)
directly to `mail`.

### Plain-text part

Add a `welcome.text.slv` next to `welcome.html.slv` and it's rendered
automatically as the `text/plain` alternative — no extra code:

```erb
<!-- app/views/user_mailer/welcome.text.slv -->
Welcome, <%= user.name %>! Visit https://example.com/start to get going.
```

### Multiple arguments

An action can take more than one argument:

```soli
class OrderMailer < Mailer
  def receipt(order, invoice)
    @order = order
    this.mail(to: order.email, subject: "Receipt #{invoice.number}")
  end
end

OrderMailer.receipt(order, invoice).deliver_later
```

Omitted arguments arrive as `nil` (default parameter values are not applied),
so pass every argument explicitly or fold them into a single hash.

## Previewing (dev)

With `soli serve --dev`, browse every mailer view at **`/__soli/mailers`** — a
gallery listing each `<mailer>/<action>` and rendering its HTML body in an
iframe, so you can iterate on an email without sending one. Each entry links to
**`/__soli/mailers/<mailer>/<action>`** for a full-page preview.

Because the preview renders the view directly (not the action), give it example
data with a leading `<%# preview: {json} %>` header — the same convention as the
[component catalog](views.md#component-catalog-dev):

```erb
<%# preview: { "user": { "name": "Ada Lovelace" } } %>
<h1>Welcome, <%= h(user.name) %>!</h1>
```

Previews render the HTML part only (no layout) with built-in helpers plus the
`preview` data; the action's real instance variables and request context aren't
available. Views with no `preview` header still list, but show a render error
where they reference missing locals. The gallery is dev-only — the routes don't
exist in production.

## Attachments

Pass an `attachments` array to `mail`, or chain `attach` / `attach_base64` on the
returned `Message`. Text content goes verbatim; binary content is supplied as a
base64 string:

```soli
UserMailer.welcome(user)
  .attach("notes.txt", "Thanks for signing up!")
  .attach_base64("logo.png", Base64.encode(File.read("logo.png")), "image/png")
  .deliver_later
```

Each attachment is `{ "filename", "content_type", "content" }` (text) or
`{ "filename", "content_type", "base64" }` (binary).

## Sending

Every action returns a `Message`. Send it now, or enqueue it on the
[Job queue](builtins/jobs.md) for background delivery:

```soli
UserMailer.welcome(user).deliver_now     # send synchronously, in-request
UserMailer.welcome(user).deliver_later   # enqueue; returns immediately
```

`deliver_later` enqueues a `__MailDelivery` job; if the queue is unavailable it
logs and falls back to sending synchronously so a message is never dropped.

## `this.mail(...)`

| Argument      | Type        | Notes |
|---------------|-------------|-------|
| `to`          | String/Array | One address or a list. |
| `subject`     | String      | Encoded automatically for non-ASCII. |
| `html`        | String?     | Skip the view render and use this body. |
| `text`        | String?     | Plain-text alternative part. |
| `cc`, `bcc`   | String/Array? | `bcc` stays out of the headers. |
| `reply_to`    | String?     | |
| `sender`      | String?     | Overrides the configured default `From`. |
| `attachments` | Array?      | `{ "filename", "content_type", "content" }` hashes. |
| `template`    | String?     | Override the convention view path. |

Addresses accept either a bare `addr@host` or a `"Name <addr@host>"` display
form.

## Configuration

Configure delivery once in `config/application.sl`:

```soli
Mailer.configure({
  "delivery_method": "smtp",       # "smtp" | "test" | "logger"
  "host": getenv("SMTP_HOST"),
  "port": 587,                     # 465 = implicit TLS, 587 = STARTTLS
  "user": getenv("SMTP_USER"),
  "pass": getenv("SMTP_PASS"),
  "tls": "auto",                   # "auto" | "starttls" | "tls" | "none"
  "from": "Acme <noreply@example.com>"
})
```

`tls: "auto"` uses implicit TLS on port 465 and STARTTLS everywhere else.
Authentication (`AUTH LOGIN`) is used when `user`/`pass` are set.

### Delivery methods

| Method   | Behavior |
|----------|----------|
| `smtp`   | Send over SMTP (default). |
| `test`   | Capture mail in memory for assertions — never sends. |
| `logger` | Print a one-line summary instead of sending. |

### Environment variables

| Variable | Maps to |
|----------|---------|
| `SOLI_MAIL_DELIVERY_METHOD` | `delivery_method` |
| `SOLI_SMTP_HOST` / `SOLI_SMTP_PORT` | `host` / `port` |
| `SOLI_SMTP_USER` / `SOLI_SMTP_PASS` | `user` / `pass` |
| `SOLI_SMTP_TLS` | `tls` |
| `SOLI_SMTP_FROM` | `from` |
| `SOLI_SMTP_DOMAIN` | EHLO name / `Message-ID` host |

## Testing

Set `delivery_method: "test"` and assert on `Mailer.deliveries()`:

```soli
describe("UserMailer", fn() {
  before_each(fn() {
    Mailer.configure({ "delivery_method": "test" })
    Mailer.clear_deliveries()
  })

  test("welcome email", fn() {
    UserMailer.welcome(user).deliver_now()
    let sent = Mailer.deliveries()
    assert_eq(len(sent), 1)
    assert_eq(sent[0]["to"], user.email)
  })
})
```

`Mailer.deliveries()` returns the rendered mail hashes (newest last);
`Mailer.clear_deliveries()` resets the buffer.
