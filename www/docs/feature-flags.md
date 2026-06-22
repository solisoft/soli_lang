# Feature Flags

Toggle features on and off at runtime — no redeploy. Flags live in the shared
cache (SoliKV), so every worker sees the same value and you can flip a feature
live, ramp it out to a percentage of users, or kill it instantly when something
goes wrong.

Feature flags ship as a stdlib module in every new app at
`stdlib/feature_flags.sl`. Import it where you need it:

```soli
import "../../stdlib/feature_flags.sl"
```

## Quick start

```soli
# In a controller action
if FeatureFlags.enabled?("checkout_v2", user: current_user["id"])
  return render("checkout/v2")
end

return render("checkout/v1")
```

Manage flags from a console, an admin action, or `config/application.sl` at boot:

```soli
FeatureFlags.enable("checkout_v2")            # on for everyone
FeatureFlags.disable("checkout_v2")           # global kill-switch
FeatureFlags.set_rollout("checkout_v2", 25)   # 25% of users (stable per user)
FeatureFlags.enable_for("checkout_v2", "u_42")     # allowlist a single user
FeatureFlags.enable_group("checkout_v2", "beta")   # allowlist a group
```

## How a flag is evaluated

`FeatureFlags.enabled?(name, user:, groups:)` resolves in this order — the first
rule that matches wins:

1. **Environment override.** If `SOLI_FEATURE_<NAME>` is set, it decides the
   answer outright and the cache is never touched (see below).
2. **Unknown flag.** A flag that was never stored is **off**.
3. **Kill-switch.** A flag whose `on` is `false` is **off**, regardless of any
   rollout or allowlist.
4. **Allowlists.** If `user` is on the flag's user allowlist, or any of `groups`
   is on its group allowlist, the flag is **on** — allowlists always beat the
   rollout percentage.
5. **Percentage rollout.** If a rollout is set below 100%, the user is bucketed
   deterministically (see below). Anonymous traffic (`user` omitted) is never in
   a partial rollout.
6. **Plain on.** Otherwise the flag's `on` value applies.

## Percentage rollout

`set_rollout(name, percent)` turns a flag on for a stable slice of your users.
Membership is derived from a hash of the flag name and the user id, so a given
user always gets the same answer — and as you ramp the percentage up, users who
were already in stay in:

```soli
FeatureFlags.set_rollout("new_dashboard", 10)   # 10% see it
# …later, once it looks healthy…
FeatureFlags.set_rollout("new_dashboard", 50)   # the original 10% are still in
FeatureFlags.set_rollout("new_dashboard", 100)  # everyone
```

Because bucketing is keyed on the user id, a logged-out visitor (`user` omitted)
is never counted in a partial rollout — pass a stable id when you want anonymous
visitors bucketed too.

## Environment overrides

`SOLI_FEATURE_<NAME>` forces a flag without touching the cache — ideal for CI,
local development, and emergency kill-switches. The env name is `SOLI_FEATURE_`
plus the flag name upper-cased:

```bash
SOLI_FEATURE_CHECKOUT_V2=1   # force on
SOLI_FEATURE_CHECKOUT_V2=0   # force off
```

Truthy values are `1`, `true`, `on`, `yes` (case-insensitive); anything else
reads as off. Keep flag names identifier-like (letters, digits, underscores) so
they map cleanly to environment variable names.

## Fail-safe behavior

Reads never throw on the request hot path. If the cache is unreachable,
`enabled?` treats the flag as unconfigured and returns **off** (after honoring
any env override). A cache outage degrades features gracefully instead of
500-ing the request.

Flags persist far beyond the cache's default 1-hour TTL — by default ~10 years,
effectively permanent. Override with `SOLI_FEATURE_TTL` (in seconds).

## Storage format

Each flag is a small config hash stored under the cache key `feature:<name>`:

```soli
{ "on": true, "rollout": 25, "users": ["u_42"], "groups": ["beta"] }
```

Read it with `FeatureFlags.get(name)` (returns `null` if the flag was never set)
and remove it with `FeatureFlags.clear(name)`.

## API reference

| Method | Description |
|--------|-------------|
| `FeatureFlags.enabled?(name, user:, groups:)` | Is the flag on for this user/groups? Returns a `Bool`; fails safe to `false`. |
| `FeatureFlags.enable(name)` | Turn the flag on for everyone. |
| `FeatureFlags.disable(name)` | Global kill-switch — off for everyone. |
| `FeatureFlags.set_rollout(name, percent)` | Turn on for a stable `percent` (0–100) of users. |
| `FeatureFlags.enable_for(name, user)` | Add a user to the allowlist (beats the rollout %). |
| `FeatureFlags.enable_group(name, group)` | Add a group to the allowlist. |
| `FeatureFlags.get(name)` | Raw config hash, or `null` if unset. |
| `FeatureFlags.clear(name)` | Forget the flag (env overrides still apply). |

## Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `SOLI_FEATURE_<NAME>` | Force flag `<NAME>` on/off, bypassing the cache | unset |
| `SOLI_FEATURE_TTL` | How long stored flags live, in seconds | `315360000` (~10 years) |

Flag storage uses the same SoliKV connection as [Cache](/docs/builtins/cache)
(`SOLIKV_RESP_HOST`, `SOLIKV_RESP_PORT`, `SOLIKV_TOKEN`).
