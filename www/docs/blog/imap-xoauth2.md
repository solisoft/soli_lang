# Reading a Mailbox in 2026: XOAUTH2, UIDs, and Fetching Only the Headers

A support address is one of the most ordinary things an application reads. Mail
arrives at `support@`, something opens a ticket, the sender gets an answer that
says so, and the message is filed where the next run will not trip over it again.
Soli has had an `Imap` client for a while. Point that job at a Google Workspace
mailbox with the client as it was, and it runs into four problems, each of which
needed a change in the client:

1. **It could not log in.** The domain's administrator had switched off app
   passwords, so `LOGIN` had no credential the server would take.
2. **Listing was expensive.** The only way to see what had arrived downloaded
   every message whole, attachments included.
3. **The mutating verbs took positions.** "Mark this seen" and "move this" took
   sequence numbers, and a sequence number changes the moment anything before it
   leaves the mailbox.
4. **The answer did not thread.** A reply without `In-Reply-To` and `References`
   shows up in the customer's mail client as a new conversation.

This post walks through the job that came out of it: poll the headers, fetch a
body only for mail that is actually new, reply in the same thread, and move what
has been handled.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/imap-xoauth2.svg" width="1024" height="576" alt="Three steps of a support-inbox job. First, LOGIN with an app password is refused while AUTHENTICATE XOAUTH2 with an access token succeeds. Second, fetch_headers_set(&quot;102:*&quot;) sends one UID FETCH asking only for SUBJECT FROM TO DATE, RFC822.SIZE and BODYSTRUCTURE, returning UID 102 with no attachment and UID 103 with one. Third, UID 102 is fetched, answered with In-Reply-To, marked seen and moved to Handled, while UID 103 is moved to Needs a human. Underneath, the sequence-number trap: before the move sequence number 2 is UID 102, after it sequence number 2 is UID 103." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Authenticate with a token, list with headers only, then act on UIDs, because a sequence number moves as soon as a message leaves.</figcaption>
</figure>

## When `LOGIN` stops working

For years the advice for reading Gmail over IMAP was an
[app password](https://support.google.com/accounts/answer/185833): a
separate secret you pass to `LOGIN` instead of the account's real password. A
Google Workspace administrator can turn app passwords off for the whole domain,
and by default now does. After that, `LOGIN` has nothing it
will accept, so there is no way into the mailbox over IMAP without
`AUTHENTICATE XOAUTH2`.

Soli supports that now. Pass an OAuth **access token** as `xoauth2` and leave the
password empty:

```soli
mailbox = Imap.new("imap.gmail.com", "support@acme.example", "", { "xoauth2": access_token })
```

The client sends `AUTHENTICATE XOAUTH2` with the base64 of
`user=…\x01auth=Bearer …\x01\x01` as the initial response, all in one command.

Soli takes only the short-lived access token. It never sees the refresh token, and
it does not obtain one for you: there is no built-in "log this mailbox in"
helper. You keep the refresh token and trade it for an access token each time the
job runs. That trade is a single form POST, and the
`soli generate oauth google` scaffold already makes the same kind of request with
`HTTP.request` in `app/services/google_oauth.sl`:

```soli
# app/services/google_mail_token.sl
class GoogleMailToken {
  # Trades the stored refresh token for a short-lived access token.
  static def fresh() -> String {
    form = "client_id=" + url_encode(getenv("GOOGLE_CLIENT_ID")) +
      "&client_secret=" + url_encode(getenv("GOOGLE_CLIENT_SECRET")) +
      "&refresh_token=" + url_encode(getenv("SUPPORT_REFRESH_TOKEN")) +
      "&grant_type=refresh_token"
    response = HTTP.request("POST", "https://oauth2.googleapis.com/token",
      { "Content-Type": "application/x-www-form-urlencoded" }, form)
    unless response["status"] == 200
      throw "token refresh failed (HTTP #{response["status"]}): #{response["body"]}"
    end
    json_parse(response["body"])["access_token"]
  }
}
```

You get the refresh token once, when someone consents. The generated sign-in
flow is the wrong tool for that as it stands: it asks for `openid email profile`
with `access_type=online`, which is enough to identify a user but does not give
mailbox access or a refresh token. A mailbox needs Google's mail scope
(`https://mail.google.com/`) and offline access. See
[OAuth Client](/docs/security/oauth-client) and the
[Google OAuth walkthrough](/docs/blog/google-oauth) for the flow itself.

A rejected token takes more handling than a bad password. The server does not
answer `NO` straight away. It sends a continuation (`+` followed by base64 JSON
describing the problem) and waits for the client to acknowledge it with an empty
line. Only then does it send the tagged `NO`. A client that does not know about
this step waits until the socket times out. Soli's client answers the `+` itself,
because its general response reader would treat the `+` as untagged output and
block. It accepts one continuation and refuses a second rather than looping. What
reaches your code is an ordinary error:

```
IMAP authentication failed: XOAUTH2 refused (NO): …
```

That error is the signal to refresh the token, not to retry with the same one.

## Listing without downloading

`fetch(seq)` and `fetch_uid(uid)` send `BODY.PEEK[]`, which is the whole message:
every part and every attachment. That is right when you are about to read a
message. It is wasteful when all you need is to know what has arrived, because
a customer's photographed invoice comes down the wire just so a loop can look at
the subject line. The `PEEK` does mean that reading a message never marks it
`\Seen` as a side effect. You decide when to do that.

The `fetch_headers` family asks for much less. Every variant sends the same item
list:

```
(UID FLAGS BODY.PEEK[HEADER.FIELDS (SUBJECT FROM TO DATE)] RFC822.SIZE BODYSTRUCTURE)
```

It returns four header lines per message, parsed by the same code into the same
hash as a full fetch. `text_body` and `html_body` are `null` because they were
never requested. Two extra fields tell you what the full download would have
cost:

| Field | Meaning |
|-------|---------|
| `bytes` | The message's real size, from `RFC822.SIZE`. `size` is the length of what came back, which here is only the header block. |
| `clips` | How many parts declare themselves attachments, read off `BODYSTRUCTURE`. None of them is downloaded. |

The difference between the two sizes is easy to see. In the test mailbox used for
this post, a short message with an 8 KB PDF attached lists as `size` 122,
`bytes` 8413, `clips` 1.

There are four ways to call it: `fetch_headers(seq)` and `fetch_headers_uid(uid)`
for one message, `fetch_headers_range(lo, hi)` for a run of sequence numbers, and
`fetch_headers_set(set)` for a UID set such as `"100:*"` or `"1,5,9"`. The last
two save round trips rather than bytes. Twenty messages fetched one at a time
means twenty commands and twenty waits on the network to Google. `UID FETCH 100:*`
is one command and one wait.

The set is validated against the RFC 3501 sequence-set grammar (digits, `,`, `:`
and `*`) before it goes into the command. A string that could contain a space
could also contain a second IMAP command:

```
Imap.fetch_headers_set("1:* BODY[]"): a sequence set is digits, ',', ':' and '*'
```

Watch out for one IMAP rule when you poll with `n:*`. In RFC 3501, `*` means "the
largest UID in the mailbox", and a range means the same thing whichever way round
it is written. So if the largest UID is 103, `104:*` means `103:104`, and the
server returns message 103, which you have already handled. Running against the
test mailbox:

```soli
mailbox.fetch_headers_set("104:*").map(fn(row) row["uid"])   # [103]
```

A poller therefore filters on `row["uid"] > last_uid` and does not assume an
empty result means nothing is new.

## Sequence numbers are positions

IMAP gives every message two numbers. The **sequence number** is its position in
the mailbox right now: 1 to however many messages there are. The **UID** is an
identifier that stays fixed as long as the mailbox's `UIDVALIDITY` does not
change. Until this release, every mutating verb in the client (`mark_seen`,
`mark_unseen`, `delete`, `copy`, `move`) took the position.

That goes wrong the moment anything moves. Here is the test mailbox with UIDs
101, 102 and 103:

```soli
mailbox.fetch_headers(2)["uid"]    # 102
mailbox.uid_move(102, "Handled")
mailbox.fetch_headers(2)["uid"]    # 103
```

After the move, position 2 is a different message. A loop that collected
sequence numbers first and then moved and flagged them one by one would act on
the wrong mail from the second iteration onwards, and neither the server nor the
client would report an error. The request is valid; it just names a different
message.

A program that keeps any state, such as a ticket table or a cursor, stores UIDs,
because UIDs are the numbers that stay put. Before these methods existed, each
mutation meant turning the UID into a position first with a `SEARCH UID n`. That
is one extra round trip, measured at about 200 ms against Gmail, and the second
command still depended on nothing having moved in between. The new verbs do the
same operations addressed by UID, in one command:

| By position | By UID | On the wire |
|-------------|--------|-------------|
| `mark_seen(seq)` / `mark_unseen(seq)` | `uid_mark_seen(uid)` / `uid_mark_unseen(uid)` | `UID STORE n ±FLAGS (\Seen)` |
| `delete(seq)` | `uid_delete(uid)` | `UID STORE n +FLAGS (\Deleted)` |
| `copy(seq, box)` | `uid_copy(uid, box)` | `UID COPY n "box"` |
| `move(seq, box)` | `uid_move(uid, box)` | `UID MOVE n "box"` |

`uid_delete` only sets the flag, like `delete` does. The message is removed on
`expunge()`.

A UID is only meaningful together with the mailbox's `UIDVALIDITY`. `select()`
returns it, and if it changes, every UID you stored for that mailbox refers to
nothing. A cursor should store both:

```soli
# app/models/inbox_cursor.sl
class InboxCursor < Model
  # The last UID handled, and the UIDVALIDITY it belongs to.
  static def for_mailbox(name: String, uidvalidity: Int) {
    cursor = InboxCursor.find_by("mailbox", name)
    if cursor.nil?
      return InboxCursor.create({ "mailbox": name, "uidvalidity": uidvalidity, "last_uid": 0 })
    end
    # A new UIDVALIDITY means every UID stored under the old one names nothing.
    cursor.update({ "uidvalidity": uidvalidity, "last_uid": 0 }) if cursor.uidvalidity != uidvalidity
    cursor
  }
end
```

## The job

With those pieces, the poller is short. It runs every two minutes from cron,
lists everything past the cursor in one round trip, sends anything with an
attachment or over 2 MB to a person, and answers the rest:

```soli
# app/jobs/support_inbox_job.sl
class SupportInboxJob {
  static cron: String = Cron.every("2 minutes")

  static def perform(args: Hash) {
    token   = GoogleMailToken.fresh()
    mailbox = Imap.new("imap.gmail.com", getenv("SUPPORT_ADDRESS"), "", { "xoauth2": token })
    status  = mailbox.select("INBOX")
    cursor  = InboxCursor.for_mailbox("INBOX", status["uidvalidity"])

    # Everything past the high-water mark, headers only, in one round trip.
    # `n:*` still answers the newest message when nothing is past n.
    rows = mailbox.fetch_headers_set("#{cursor.last_uid + 1}:*")
      .filter(fn(row) row["uid"] > cursor.last_uid)

    for row in rows
      # Claimed before acting: a crash below leaves the message unread in the
      # INBOX for a person, rather than answered twice by the next run.
      cursor.update({ "last_uid": row["uid"] })

      if row["clips"] > 0 || row["bytes"] > 2_000_000
        mailbox.uid_move(row["uid"], "Needs a human")
      else
        message = mailbox.fetch_uid(row["uid"])
        SupportReply.answer(message)
        mailbox.uid_mark_seen(row["uid"])
        mailbox.uid_move(row["uid"], "Handled")
      end
    end

    mailbox.logout()
  }
}
```

Against the test mailbox, with the cursor at 101, one run sent exactly these
commands:

```
a0001 AUTHENTICATE XOAUTH2 dXNlcj1zdXBwb3J0QGV4YW1wbGUuY29tAWF1dGg9QmVhcmVyIGdvb2QtdG9rZW4BAQ==
a0002 SELECT "INBOX"
a0003 UID FETCH 102:* (UID FLAGS BODY.PEEK[HEADER.FIELDS (SUBJECT FROM TO DATE)] RFC822.SIZE BODYSTRUCTURE)
a0004 UID FETCH 102 (UID FLAGS BODY.PEEK[])
a0005 UID STORE 102 +FLAGS (\Seen)
a0006 UID MOVE 102 "Handled"
a0007 UID MOVE 103 "Needs a human"
a0008 LOGOUT
```

The message with an attachment was listed and moved, and its body never came
down the wire. Only the message the job answered was fetched in full.

The cursor is updated *before* the reply, and that ordering is deliberate. There
are two ways this job can fail, and they are not equally bad. If the cursor moves
after the reply, a crash between the two gives the customer a second copy of the
same automated answer on the next run. If the cursor moves first, the same crash
leaves the message unread in the INBOX, where a person will see it. The job
chooses at most one reply over at least one reply.

## Replying so it threads

Mail clients group messages into conversations using two headers. `In-Reply-To`
names the message being answered. `References` is the parent's own `References`
with the parent's `Message-ID` appended. Without them, a reply appears in the
customer's mail client as a new conversation that happens to start with "Re:".

`Mailer.deliver` now accepts a `headers` hash for exactly this. The values have to
come from the original message, and the headers-only fetch does not include
them: it asks for `SUBJECT FROM TO DATE` and nothing else. The job gets them from
the full fetch it makes anyway before answering, whose `raw` field holds the
complete source:

```soli
# app/services/support_reply.sl
class SupportReply {
  static def answer(message: Hash) {
    head       = message["raw"].split("\r\n\r\n")[0]
    message_id = SupportReply.header(head, "Message-ID")
    references = SupportReply.header(head, "References")

    ticket = Ticket.create({
      "from":    message["from"]["address"],
      "subject": message["subject"],
      "body":    SupportReply.best_text(message)
    })

    ticket_url = "https://support.example.com/tickets/#{ticket.id}"
    source = [
      "Hi #{message["from"]["name"] || "there"},",
      "",
      "We have your message and opened **ticket #{ticket.id}** for it.",
      "Someone from the team will answer within one working day.",
      "",
      "- Reply to this email to add to the ticket",
      "- Follow it at [your support page](#{ticket_url})"
    ].join("\n")
    subject = message["subject"].to_s
    subject = "Re: " + subject unless subject.downcase().starts_with?("re:")

    Mailer.deliver({
      "from":         "Acme Support <support@example.com>",
      "to":           message["from"]["address"],
      "subject":      subject,
      "text":         Markdown.to_text(source),
      "html":         Markdown.to_html(source),
      "alternatives": [ { "content_type": "text/markdown", "body": source } ],
      "headers":      {
        "In-Reply-To": message_id,
        "References":  [references, message_id].compact().join(" ")
      }
    })
  }

  # The source the sender typed, when the message carries it.
  static def best_text(message: Hash) -> String {
    markdown = message["parts"].find(fn(part) part["content_type"] == "text/markdown")
    return markdown["body"] if markdown
    message["text_body"] || strip_html(message["html_body"].to_s)
  }

  # One header's value, unfolded; nil when the message does not carry it.
  static def header(head: String, name: String) {
    found = Regex.capture("(?im)^#{name}:(?P<value>.*(?:\\r?\\n[ \\t].*)*)", head)
    found.nil? ? nil : Regex.replace_all("\\s+", found["value"], " ").trim()
  }
}
```

The `header` helper handles two details. It searches only the header block (up
to the first blank line), so a quoted `References:` line in the body of the
message cannot match. It also unfolds: a long `References` value is usually
split across several lines, each continuation starting with whitespace, and
those lines need to be joined back into one value.

`headers` is the only field where an application writes both the name and the
value of a header line, so the mailer checks both. A name or value containing CR
or LF is header injection and is refused, as is a name containing `:` or
whitespace:

```
mail `In-Reply-To` contains a forbidden control character (CR, LF, or NUL)
```

The check runs when the MIME message is built, which only happens on the `smtp`
path. `delivery_method: "test"` stores the hash exactly as given and never builds
it, so a spec that captures deliveries will not catch an injected header. Test
the guard against SMTP, or validate the values yourself.

## Three versions of the answer

The reply is written once, as Markdown, and sent in three forms: HTML for clients
that render it, plain text for those that do not, and the Markdown source itself
as a `text/markdown` part through `alternatives`. The mailer orders the parts from
least to most rich (plain text, then the extras in the order given, then HTML)
because a mail client shows the last part it understands (RFC 2046 §5.1.4). Most
clients show the HTML and ignore the source. A client that reads
[`parts`](/docs/builtins/imap) can use the Markdown instead, and
`best_text` above does that on the receiving side: if a customer's own client
sent Markdown, that is what goes into the ticket.

The plain-text version is where the obvious shortcut goes wrong. Running
`strip_html` over the HTML does not give the same result as `Markdown.to_text`.
Here is the same refund note through each, starting with
`Markdown.to_text(source)`:

```
Your refund for order #4821 went out today. It usually lands in:

- 3 to 5 working days by card
- 1 day by bank transfer

Track it at your account <https://shop.example.com/orders/4821>.
```

`strip_html(Markdown.to_html(source))`:

```
Your refund for order #4821 went out today. It usually lands in:

3 to 5 working days by card
1 day by bank transfer

Track it at your account.
```

Stripping tags drops the list markers, and it also drops the link address: the
reader is told to track the refund at "your account" and given nowhere to go.
`Markdown.to_text` keeps both. A link becomes `text <url>` unless the text is
already the URL, list items keep `- ` or `1. ` indented two spaces per level,
quotes keep `> `, code fences are kept as written, table cells are joined by
` | `, and emphasis loses its markers.

## What this does not do

- **It polls.** The client does not implement `IDLE`. New mail is found when cron
  next runs the job, so the delay is set by the schedule.
- **Token acquisition is yours.** `xoauth2` takes a token. Getting consent,
  storing the refresh token and refreshing it all happen in your application.
- **Server responses are size-limited.** A single IMAP literal (one message
  body, for example) over 32 MiB is refused rather than allocated. Raise the
  limit with `SOLI_IMAP_MAX_LITERAL_BYTES` if you really do need to read mail
  that large, but a support job should probably route such messages to a person
  from the headers-only listing instead.

The full method list, the message hash, and the search syntax are in the
[IMAP reference](/docs/builtins/imap). The mailer's `headers`
and `alternatives` are in [Mailer](/docs/builtins/mailer).
