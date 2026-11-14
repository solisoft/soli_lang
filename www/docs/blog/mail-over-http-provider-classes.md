# Mail over HTTP: one class per provider

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/mail-over-http-provider-classes.svg" width="1024" height="576" alt="A Soli mailer hands one JSON envelope to a provider class, which reshapes it for Brevo, Resend, Postmark, SendGrid or Mailgun." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
</figure>

Soli's mailers have sent over SMTP since the first release. SMTP still works,
but it is rarely what a provider recommends any more. Brevo, Resend, Postmark,
SendGrid and Mailgun all put an HTTP API first: one request per message, an
error that says what is wrong, no port 587 to open. And some places cannot
speak SMTP at all — a Cloudflare Worker can make HTTP requests, not open a TCP
connection to a mail server.

So mail now has a second road: `delivery_method: "http"`. This post covers
how it works, and gives a ready-made class for each of the five big providers
— copy one into `app/mailers/` and you are sending.

## One envelope, POSTed

Your mailers do not change. `deliver_now`, `deliver_later` and
`Mailer.deliver` build the same message; with the `http` method, Soli POSTs it
as one JSON shape:

```json
{
  "from": "Acme <noreply@example.com>",
  "to": ["Ana Vieira <ana@example.com>"], "cc": [], "bcc": [], "reply_to": null,
  "subject": "Your order", "text": "…", "html": "<p>…</p>",
  "headers": {"X-Order": "42"},
  "attachments": [{"filename": "invoice.pdf", "content_type": "application/pdf", "content_base64": "JVBERi0…"}],
  "message_id": "…@host",
  "mime": "From: …"
}
```

`mime` is the complete message the SMTP road would have sent, for APIs that
take raw MIME. If your provider — or your own relay — accepts that shape,
pointing Soli at it is all there is:

```soli
Mailer.configure({
  "delivery_method": "http",
  "url": "https://mail-relay.internal.example.com/send",
  "api_key": getenv("MAIL_API_KEY"),   # sent as Authorization: Bearer <key>
  "from": "Acme <noreply@example.com>"
})
```

A 2xx answer is a delivery. Anything else raises an error that carries the
status and the start of the provider's answer — `mail POST
https://api.brevo.com/v3/smtp/email returned HTTP 400: {"code":"invalid_parameter",…}`
— and never the key or the URL's query string. `deliver_later` retries it like
any other job.

## The request hook

Real providers each want their own shape: `sender` as an object, `To` as a
comma-joined string, `personalizations`, multipart. Rather than freeze five
adapters into the binary, Soli gives you the last word: name a class of your
app in `request`, and its `request(mail, req)` receives the standard request —
`{"url", "headers", "body": <the envelope>}` — and returns the one to send. A
hash body goes out as JSON, a string as it is.

```soli
Mailer.configure({"delivery_method": "http", "request": "BrevoMail",
                  "api_key": getenv("BREVO_API_KEY"), "from": "Acme <noreply@example.com>"})
```

Why a class and not a lambda? `Mailer.configure` runs once, at boot, in
`config/application.sl`, and the configuration every worker reads cannot carry
a closure: a function made in one interpreter does not exist in another. A
class in `app/mailers/` is loaded by every worker, so the name always resolves.

With a hook, `url` is optional: each class below knows its provider's endpoint,
and a configured `url` only replaces the base — for an EU region, a proxy, or a
test server.

Each class reads the key from the standard request
(`Authorization: Bearer <key>`) and puts it where its provider wants it. They
are plain Soli: change whatever your provider account needs (tags, tracking,
templates).

## Brevo

Addresses as `{"name", "email"}` objects, the key in an `api-key` header, the
attachments under `attachment` (singular).

```soli
# Brevo — https://developers.brevo.com/reference/sendtransacemail
#
#   Mailer.configure({"delivery_method": "http", "request": "BrevoMail",
#                     "api_key": getenv("BREVO_API_KEY")})
#
# A configured `url` replaces https://api.brevo.com (a proxy, a test server).
class BrevoMail
  static def request(mail, req)
    envelope = req["body"]
    api_key = req["headers"]["Authorization"].to_s.replace("Bearer ", "")
    base_url = req["url"].blank? ? "https://api.brevo.com" : req["url"]
    body = {
      "sender": BrevoMail.address(envelope["from"]),
      "to": envelope["to"].map { |a| BrevoMail.address(a) },
      "subject": envelope["subject"]
    }
    body["cc"] = envelope["cc"].map { |a| BrevoMail.address(a) } if envelope["cc"].length > 0
    body["bcc"] = envelope["bcc"].map { |a| BrevoMail.address(a) } if envelope["bcc"].length > 0
    body["replyTo"] = BrevoMail.address(envelope["reply_to"]) unless envelope["reply_to"].nil?
    body["textContent"] = envelope["text"] unless envelope["text"].nil?
    body["htmlContent"] = envelope["html"] unless envelope["html"].nil?
    body["headers"] = envelope["headers"] if envelope["headers"].length > 0
    if envelope["attachments"].length > 0
      body["attachment"] = envelope["attachments"].map { |a|
        {"name": a["filename"], "content": a["content_base64"]}
      }
    end
    {
      "url": base_url + "/v3/smtp/email",
      "headers": {"api-key": api_key, "Content-Type": "application/json", "Accept": "application/json"},
      "body": body
    }
  end

  # "Ana Vieira <ana@example.com>" → {"name": "Ana Vieira", "email": "ana@example.com"}
  static def address(text)
    parts = text.split("<")
    return {"email": text.trim} if parts.length < 2

    name = parts[0].trim.replace("\"", "")
    email = parts[1].replace(">", "").trim
    name.blank? ? {"email": email} : {"name": name, "email": email}
  end
end
```

## Resend

The closest to Soli's envelope: `from`, `to`, `reply_to` take `Name <email>`
strings as they are, and the key is a Bearer token.

```soli
# Resend — https://resend.com/docs/api-reference/emails/send-email
#
#   Mailer.configure({"delivery_method": "http", "request": "ResendMail",
#                     "api_key": getenv("RESEND_API_KEY")})
#
# Resend takes "Name <email>" strings as they are; the key goes as a Bearer
# token, which is already how the standard request sends it.
class ResendMail
  static def request(mail, req)
    envelope = req["body"]
    base_url = req["url"].blank? ? "https://api.resend.com" : req["url"]
    body = {"from": envelope["from"], "to": envelope["to"], "subject": envelope["subject"]}
    body["cc"] = envelope["cc"] if envelope["cc"].length > 0
    body["bcc"] = envelope["bcc"] if envelope["bcc"].length > 0
    body["reply_to"] = envelope["reply_to"] unless envelope["reply_to"].nil?
    body["text"] = envelope["text"] unless envelope["text"].nil?
    body["html"] = envelope["html"] unless envelope["html"].nil?
    body["headers"] = envelope["headers"] if envelope["headers"].length > 0
    if envelope["attachments"].length > 0
      body["attachments"] = envelope["attachments"].map { |a|
        {"filename": a["filename"], "content": a["content_base64"]}
      }
    end
    {
      "url": base_url + "/emails",
      "headers": {"Authorization": req["headers"]["Authorization"], "Content-Type": "application/json"},
      "body": body
    }
  end
end
```

## Postmark

PascalCase fields, recipients joined with commas, the key in
`X-Postmark-Server-Token`. Postmark only sends from a confirmed Sender
Signature.

```soli
# Postmark — https://postmarkapp.com/developer/api/email-api
#
#   Mailer.configure({"delivery_method": "http", "request": "PostmarkMail",
#                     "api_key": getenv("POSTMARK_SERVER_TOKEN")})
#
# `From` must be a confirmed Sender Signature on your Postmark server.
class PostmarkMail
  static def request(mail, req)
    envelope = req["body"]
    token = req["headers"]["Authorization"].to_s.replace("Bearer ", "")
    base_url = req["url"].blank? ? "https://api.postmarkapp.com" : req["url"]
    body = {"From": envelope["from"], "To": envelope["to"].join(", "), "Subject": envelope["subject"]}
    body["Cc"] = envelope["cc"].join(", ") if envelope["cc"].length > 0
    body["Bcc"] = envelope["bcc"].join(", ") if envelope["bcc"].length > 0
    body["ReplyTo"] = envelope["reply_to"] unless envelope["reply_to"].nil?
    body["TextBody"] = envelope["text"] unless envelope["text"].nil?
    body["HtmlBody"] = envelope["html"] unless envelope["html"].nil?
    if envelope["headers"].length > 0
      body["Headers"] = envelope["headers"].keys.map { |name| {"Name": name, "Value": envelope["headers"][name]} }
    end
    if envelope["attachments"].length > 0
      body["Attachments"] = envelope["attachments"].map { |a|
        {"Name": a["filename"], "Content": a["content_base64"], "ContentType": a["content_type"]}
      }
    end
    {
      "url": base_url + "/email",
      "headers": {
        "X-Postmark-Server-Token": token,
        "Content-Type": "application/json",
        "Accept": "application/json"
      },
      "body": body
    }
  end
end
```

## SendGrid

Recipients live under `personalizations`, and the bodies under `content`, one
part per MIME type — plain text first, then HTML.

```soli
# SendGrid — https://www.twilio.com/docs/sendgrid/api-reference/mail-send/mail-send
#
#   Mailer.configure({"delivery_method": "http", "request": "SendgridMail",
#                     "api_key": getenv("SENDGRID_API_KEY")})
#
# EU regional subusers: add "url": "https://api.eu.sendgrid.com".
class SendgridMail
  static def request(mail, req)
    envelope = req["body"]
    base_url = req["url"].blank? ? "https://api.sendgrid.com" : req["url"]
    recipients = {"to": envelope["to"].map { |a| SendgridMail.address(a) }}
    recipients["cc"] = envelope["cc"].map { |a| SendgridMail.address(a) } if envelope["cc"].length > 0
    recipients["bcc"] = envelope["bcc"].map { |a| SendgridMail.address(a) } if envelope["bcc"].length > 0
    content = []
    content.push({"type": "text/plain", "value": envelope["text"]}) unless envelope["text"].nil?
    content.push({"type": "text/html", "value": envelope["html"]}) unless envelope["html"].nil?
    body = {
      "personalizations": [recipients],
      "from": SendgridMail.address(envelope["from"]),
      "subject": envelope["subject"],
      "content": content
    }
    body["reply_to"] = SendgridMail.address(envelope["reply_to"]) unless envelope["reply_to"].nil?
    body["headers"] = envelope["headers"] if envelope["headers"].length > 0
    if envelope["attachments"].length > 0
      body["attachments"] = envelope["attachments"].map { |a|
        {"content": a["content_base64"], "filename": a["filename"], "type": a["content_type"]}
      }
    end
    {
      "url": base_url + "/v3/mail/send",
      "headers": {"Authorization": req["headers"]["Authorization"], "Content-Type": "application/json"},
      "body": body
    }
  end

  # "Ana Vieira <ana@example.com>" → {"name": "Ana Vieira", "email": "ana@example.com"}
  static def address(text)
    parts = text.split("<")
    return {"email": text.trim} if parts.length < 2

    name = parts[0].trim.replace("\"", "")
    email = parts[1].replace(">", "").trim
    name.blank? ? {"email": email} : {"name": name, "email": email}
  end
end
```

## Mailgun

Mailgun's natural API is a multipart form. Rather than map every field, this
class sends the MIME message Soli already built to `messages.mime`, so
attachments, inline parts and headers go through untouched. Every recipient —
cc and bcc included — becomes a `to` field, because Mailgun delivers to the
fields, not the headers. The key goes in HTTP Basic auth, user `api`.

```soli
# Mailgun — https://documentation.mailgun.com/docs/mailgun/api-reference/send/mailgun/messages
#
#   Mailer.configure({"delivery_method": "http", "request": "MailgunMail",
#                     "api_key": getenv("MAILGUN_API_KEY")})
#
# Sends the MIME message Soli already built (`messages.mime`), so attachments,
# inline parts and headers go through untouched. The sending domain comes from
# MAILGUN_DOMAIN, else from the From address. EU domains: add
# "url": "https://api.eu.mailgun.net".
class MailgunMail
  static def request(mail, req)
    envelope = req["body"]
    api_key = req["headers"]["Authorization"].to_s.replace("Bearer ", "")
    base_url = req["url"].blank? ? "https://api.mailgun.net" : req["url"]
    domain = getenv("MAILGUN_DOMAIN") || envelope["from"].split("@").last.replace(">", "").trim
    boundary = "soli-mailgun-" + envelope["message_id"].to_s.split("@").first
    # Every recipient is a `to` field: the MIME has no Bcc header, and Mailgun
    # delivers to the fields, not the headers.
    recipients = [...envelope["to"], ...envelope["cc"], ...envelope["bcc"]]
    parts = recipients.map { |a| MailgunMail.field(boundary, "to", a) }
    parts.push("--" + boundary + "\r\n" +
      "Content-Disposition: form-data; name=\"message\"; filename=\"message.mime\"\r\n" +
      "Content-Type: message/rfc822\r\n\r\n" + envelope["mime"] + "\r\n")
    {
      "url": base_url + "/v3/" + domain + "/messages.mime",
      "headers": {
        "Authorization": "Basic " + Base64.encode("api:" + api_key),
        "Content-Type": "multipart/form-data; boundary=" + boundary
      },
      "body": parts.join("") + "--" + boundary + "--\r\n"
    }
  end

  static def field(boundary, name, value)
    "--" + boundary + "\r\nContent-Disposition: form-data; name=\"" + name + "\"\r\n\r\n" + value + "\r\n"
  end
end
```

## How these were checked

Each class lives in the Soli repository under `examples/mail_providers/`, and
`tests/mail_http_test.rs` sends one rich message — a display name, cc, bcc,
`reply_to`, a custom header and an attachment — through each of them to a local
server, then checks the exact shape its provider documents: Brevo's `sender`
and `attachment`, Postmark's `Headers` as `Name`/`Value` pairs, SendGrid's
`personalizations` and content order, Mailgun's form fields and MIME part.
They were written against each provider's API reference in November 2026; if a
provider changes its API, the class is yours to edit.

To try one against your own machine first, point `url` at a local catcher —
the URL goes through the same SSRF check as the `HTTP` class, so open the host
with `SOLI_HTTP_ALLOW_HOSTS=127.0.0.1`. In tests, keep
`delivery_method: "test"` and assert on `Mailer.deliveries()` as before: the
hook only runs on the `http` road.

## On Cloudflare Workers

The same POST is what mail needs on Cloudflare Workers, where SMTP is not an
option: with the `http` method, a mailer sends from a Worker through `fetch`,
and `deliver_later` rides Cloudflare Queues
([Mail on the edge](/docs/development-tools/edge#mail)). The full reference is
in [Mailer → Over HTTP](/docs/builtins/mailer#http).
