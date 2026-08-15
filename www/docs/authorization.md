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
- **Password reset** (`/password/reset`): hashed one-time tokens, 2-hour
  expiry, no account enumeration, emailed via the scaffolded `AuthMailer`.
- **Email confirmation** (`/confirm_email`, `/confirmation/resend`): sent on
  signup; enforcement is a one-line toggle (`auth_require_confirmed_email`
  in `app/models/user.sl`, off by default so the flow works before SMTP is
  configured).
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
  widen it. Raise the constants if your users sit behind a shared NAT.
- **Account lockout**: 10 failed logins lock the account for 30 minutes
  (auto-unlock); thresholds are constants at the top of the User model.
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
`update?`, `edit?`, `destroy?`. Inside a policy, `this.user` is the current user
and `this.record` is the record being checked.

```soli
class PostPolicy < ApplicationPolicy
  def show?
    true                                    # anyone may read a post
  end

  def update?
    return false unless this.signed_in?()   # guard the nil user first

    return this.user["_key"] == this.record["author_id"]
  end

  def destroy?
    return this.update?()                   # same rule as update
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
  def update
    post = Post.find(params["id"])
    authorize(post)                          # 403 unless PostPolicy#update? is true
    post.update(this._permit_params(params))
    return redirect(post_path(post))
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
def admin_dashboard
  forbidden("Admins only") unless current_user()&.admin?()
  return render("admin/dashboard")
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
