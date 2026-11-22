# Authorization & Policies

Soli ships a Pundit-style **policy layer** for authorization, scaffolded by
`soli generate auth` alongside session-based authentication. A policy answers
one question: *is the current user allowed to perform this action on this
record?*

## Generate it

```bash
soli generate auth
```

This writes a `User` model, the full Devise-style auth suite, and the policy
layer:

- Login/signup/logout controllers and views, plus a `load_current_user`
  middleware (which also honors remember-me cookies).
- **Sign-up policy** (`AUTH_SIGNUP`): sign-up is not open to anyone by
  default. `first_user` (the default) lets `/signup` create one account —
  the first, which becomes the admin — and then closes it; `open` lets anyone
  sign up (a public app); `closed` lets nobody (create accounts from a
  console, a seed or an admin page). Any other value counts as `closed`, so a
  typo fails shut. While sign-up is closed, `/signup` answers `403` with the
  sign-in form and the "Create one" link is hidden. **Outside development**, `first_user` asks for `AUTH_SETUP_TOKEN` (16+
  characters) on the sign-up form: on a freshly deployed app the first
  visitor to reach `/signup` became the admin. Only `soli serve --dev`, or
  `APP_ENV=development` / `test`, goes without — a deployment that forgot
  `APP_ENV=production` is still asked. With no token set, no first account is
  created, and OAuth creates none either — the admin signs up at `/signup`
  with the token.
- **A sign-in session's limits.** The remember-me cookie has a server-side
  end (`AUTH_REMEMBER_DAYS` after it was issued — a copied cookie stops
  working then, whatever the copy says), honours the gates of the sign-in form
  (confirmed address, not single sign-on only), and restores a session that
  nobody "just signed in" to: adding a passkey, an API token or an
  authenticator asks for a sign-in from the last 15 minutes
  (`AUTH_FRESH_SECONDS`). The password lockout closes the password only —
  passkeys, sign-in links and API tokens keep working, since anyone who knows
  an address can send ten wrong passwords. Small writes to the user (a
  remember-me token, a failed attempt) touch their own fields, so a sign-in
  that straddles a password reset cannot put the old password back.
- **Open sign-up gives one answer.** With `AUTH_SIGNUP=open`, `/signup` answers "check your mailbox" whether the address was free or already had an account, and signs nobody in: a new account gets a confirmation link and signs in once it is confirmed, the owner of an existing one gets a note (`AuthMailer.already_registered`). "Could not create your account" told anyone which addresses are registered — and so did signing in straight after signing up, which is why open sign-up always confirms first. `first_user` is unchanged: the first account is signed in at once.
- **Answers that take the same time.** The mails of the uniform-answer endpoints (reset, confirmation, sign-in link, lockout notice) are sent off the request (`deliver_later`), and an address with no account costs the same database write (`User.burn_write_work`) and the same Argon2 work as one that has: neither the reply nor its timing says whether an address is registered.
- **The remember-me token is used once.** Each time it restores a session it is replaced; the one it replaced stays good for a minute, for requests a browser sent side by side. Seen again later, it means two parties hold the cookie — a copy: remember-me ends for the account, **every session is ended** and the owner is mailed. The replacement is a compare-and-swap, so requests sent side by side do not lose the cookie and a password reset cannot be undone by a rotation in flight.
- **A link someone else asks for does not cancel yours.** Anyone can ask for a reset, a confirmation or a sign-in link for any address. A reset or a confirmation link sent in the last five minutes (`AUTH_RESEND_SECONDS`) stays the one that works — a new request sends nothing, with the same answer — and sign-in links stay pending side by side and asking never removes one that is waiting; a request is refused instead — silently, with the same answer — past `MAGIC_LINKS_PER_REQUESTER` (3) for one address or `MAGIC_LINKS_PENDING` (100) for the account. A browser that asks again keeps its cookie, so the link already in the mailbox still opens. The browser cookies are `__Host-magic_link` and `__Host-signup`: a sibling subdomain cannot plant them. Before, a request sent a second after the owner's replaced their link, for as long as someone kept asking.
- **One way to open a session.** Every sign-in — password, reset, link, passkey, second factor, provider, SAML, remember-me — goes through `auth_open_session(user)`, which gives the session a new id and drops what its previous occupant left: an impersonation, a SAML expiry, and the two-factor answer. That last one mattered: a browser that had signed in to an account before it turned two-factor on kept "this account has none", and signing in again with the password skipped the challenge.
- **An account nobody ever entered is taken over by the first proof of its address.** Someone can create an account with your address and their password, and wait for you to confirm it. So sign-up leaves a cookie in the browser that chose the password: a confirmation pressed there keeps it. Pressed anywhere else, for an account that was never signed in to (`User#never_signed_in?`), it revokes everything the account could be entered with and **mails** a link to choose a password — mailed, not handed to that browser, so a forwarded confirmation mail is not a way in. A sign-in link and a provider sign-in do the same. An account that has been in use is only confirmed: a mailbox alone never removes its password or its second factor. A confirmation link is dead once the account is confirmed another way.
- **Answers take the same time.** The replies that must not tell a registered address from an unknown one (reset, confirmation, sign-in link, open sign-up) are held until `AUTH_UNIFORM_SECONDS` (a quarter of a second) have passed, whatever was done behind them.
- **What counts as an account in use.** A session of its own, whenever it was opened, or an API token used on it: both stamp `signed_in_at`. An administrator impersonating does not. Do not make `auth_require_confirmed_email` false while strangers can sign up: an account signed in to at sign-up is in use from its first second, and the take-over no longer protects the address's owner.
- **Password reset** (`/password/reset`): hashed one-time tokens, 2-hour
  expiry, no account enumeration, emailed via the scaffolded `AuthMailer`.
- **Email confirmation** (`/confirm_email`, `/confirmation/resend`): sent on
  signup; enforcement is `auth_require_confirmed_email` in
  `app/models/user.sl` — on whenever sign-up is open to anyone
  (`AUTH_SIGNUP=open`), off otherwise so the flow works before SMTP is
  configured. With it on, an unconfirmed account answers a sign-in like a
  wrong password (one message, `401`): a distinct "confirm your email" reply
  appeared only once the password was right, which confirmed it. The link in
  the email shows a page with a button, and only the button confirms (a
  CSRF-checked `POST /confirm_email`): a mail scanner or a link preview that
  fetches the URL confirms nothing, so an account someone else opened with an
  address does not become "confirmed" — which is what provider and SAML
  sign-ins link an account by. Completing a password reset confirms the
  address too (the reset link proves the mailbox). Require confirmation
  whenever sign-up is open alongside SAML or a provider sign-in.
- **Account lockout**: failed attempts are counted with one atomic write
  (`increment`); a read, +1 and save let parallel guesses overwrite each
  other's count and slip past the lockout.
- **Remember-me**: an HttpOnly + Secure persistent cookie carrying a
  digest-stored token, promoted to a fresh session by the middleware and
  revoked when the password is reset.
- **Rate limiting**: sign-in, sign-up, and reset requests are throttled per
  source IP — over the limit returns `429`. The three share **one** budget
  (`rate_limiter_from_ip` keys on the address alone), so
  `AUTH_ATTEMPTS_PER_IP` per `AUTH_IP_WINDOW_SECONDS` (default 15 per 5
  minutes) is the total across all credential traffic from that address, not
  15 each. The key is the peer address, or the proxy-recorded client only
  when `enable_trust_proxy()` is on — so a rotating `X-Forwarded-For` can't
  widen it. Both numbers are read from the environment (`AUTH_ATTEMPTS_PER_IP`,
  `AUTH_IP_WINDOW_SECONDS`; unset, blank or non-positive falls back to the
  default) — raise them if your users sit behind a shared NAT.
- **Account lockout**: 10 failed logins lock the account for 30 minutes
  (auto-unlock); thresholds are constants at the top of the User model. The
  owner is mailed once when the lock trips (`AuthMailer.account_locked`). An
  account with no password — a [SAML](/docs/security/saml) `sso_only` one, a
  directory-provisioned one — counts no failed attempts: there is no password
  to guess, and counting them let anyone lock its owner out of the ways in
  that do work. SAML sign-in ignores the lock (the identity provider
  authenticated the user).
- **No account enumeration on sign-in**: an unknown address and a wrong
  password return the same message *and* take the same time — the miss path
  spends the same Argon2id work rather than returning early, so response
  timing isn't an oracle either. The "account locked" message is only shown
  once the password has been verified, so it tells the owner what happened
  without telling an attacker that the address exists.
- **Password policy**: `AUTH_MIN_PASSWORD_LENGTH` (default 12) is enforced by
  `User#password_error`, which sign-up and password-reset both call — so the
  reset flow can't be used to install a weaker password than registration
  accepts.
- `app/policies/application_policy.sl` — the `ApplicationPolicy` base class plus
  the global `authorize` / `policy_for` / `current_user` / `signed_in?` helpers.
- `app/policies/user_policy.sl` — a worked example policy.
- `app/helpers/auth_helper.sl` — `current_user` / `signed_in?` for views.

Configure SMTP (`SOLI_SMTP_*` env vars) so the reset/confirmation emails go
out — or `SOLI_MAIL_DELIVERY_METHOD=logger` in dev to print them to the
console — and set `APP_BASE_URL` to your `https://` origin, which
`auth_base_url` (`app/models/user.sl`) reads. Left unset in production, reset
and confirmation emails carry `http://localhost:5011` links.

Files in `app/policies/` are auto-loaded into the global scope at boot, like
models. **Restart the server after adding a new policy.**

## Writing a policy

A policy is a class named `<Model>Policy` that extends `ApplicationPolicy`, with
one predicate method per action: `index?`, `show?`, `create?`, `new?`,
`update?`, `edit?`, `destroy?`. Inside a policy, `@user` is the current user
and `@record` is the record being checked.

```soli
class PostPolicy < ApplicationPolicy
  def show?
    true                                    # anyone may read a post
  end

  def update?
    return false unless @signed_in?   # guard the nil user first

    @user["_key"] == @record["author_id"]
  end

  def destroy?
    @update?                         # same rule as update
  end
end
```

Policies **default to deny** — `ApplicationPolicy` returns `false` for every
predicate, so you only override what you want to allow. `new?` falls back to
`create?` and `edit?` falls back to `update?` unless you override them.

## Authorizing in a controller

Call `authorize(record)` at the top of an action. It looks up the matching
policy, builds it with the current user and record, and calls the predicate for
the **current action**. A falsey result raises a `403 Forbidden`; otherwise it
returns the record.

```soli
class PostsController < Controller
  # PATCH /posts/:id
  def update
    post = Post.find(params["id"])
    authorize(post)                          # 403 unless PostPolicy#update? is true
    post.update(@_permit_params(params))
    redirect(post_path(post))
  end
end
```

Pass an explicit action when it differs from the controller action:

```soli
authorize(post, "show")
```

A record class with **no** matching policy is denied (403), so a forgotten
policy fails closed rather than open.

## Helpers

| Helper | Scope | Returns |
|--------|-------|---------|
| `authorize(record, action?)` | controllers / policies | the record, or raises 403 |
| `policy_for(record)` | controllers / policies | the policy instance for `record` |
| `current_user()` | controllers / policies / views | the signed-in `User`, or `nil` |
| `signed_in?()` | controllers / policies / views | `true` when a user is signed in |
| `current_action()` | anywhere | the in-flight action name (`"update"`, …) |
| `forbidden(message?)` | anywhere | raises a `403 Forbidden` immediately |

`current_user` is populated per request by the `load_current_user` middleware,
which reads the user id from the session and loads the `User` record.

## Gating whole sections

Use `forbidden()` directly for coarse checks that aren't tied to a record:

```soli
# GET /admin
def admin_dashboard
  forbidden("Admins only") unless current_user()&.admin?
  render("admin/dashboard")
end
```

## Custom 403 page

A denied request is rendered through the standard production error pipeline, so
you can ship a custom `app/views/errors/403.html.slv` and it will be used
automatically — the same mechanism as the `404` page.

## See also

- [Authentication with JWT](/docs/security/authentication) — stateless tokens
- [Sessions](/docs/security/sessions) — session storage backends
- [Controllers](/docs/core-concepts/controllers) — actions, `params`, responses
