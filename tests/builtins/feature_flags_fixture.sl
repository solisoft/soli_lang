# ============================================================================
# FeatureFlags — runtime feature toggles, no redeploy required
# ============================================================================
#
# Flags are stored in the shared cache (SoliKV), so every worker sees the same
# value and you can flip a feature live. Each flag is a small config hash:
#
#   { "on": true, "rollout": 25, "users": ["u_42"], "groups": ["beta"] }
#
# Usage in a controller:
#
#   import "../../stdlib/feature_flags.sl"
#
#   def index(req: Any) -> Any
#     if FeatureFlags.enabled?("checkout_v2", user: current_user["id"])
#       return render("checkout/v2")
#     end
#     return render("checkout/v1")
#   end
#
# Managing flags (console, an admin action, or config/application.sl at boot):
#
#   FeatureFlags.enable("checkout_v2")           # on for everyone
#   FeatureFlags.disable("checkout_v2")          # global kill-switch
#   FeatureFlags.set_rollout("checkout_v2", 25)  # 25% of users (stable per user)
#   FeatureFlags.enable_for("checkout_v2", "u_42")     # allowlist a user
#   FeatureFlags.enable_group("checkout_v2", "beta")   # allowlist a group
#
# Environment override (highest priority — great for CI and emergency kills):
#
#   SOLI_FEATURE_CHECKOUT_V2=1   # force on,  bypasses the cache entirely
#   SOLI_FEATURE_CHECKOUT_V2=0   # force off, bypasses the cache entirely
#
# The env name is "SOLI_FEATURE_" + the flag name upper-cased. Keep flag names
# identifier-like (letters, digits, underscores) so they map to valid env vars.
# ============================================================================

export class FeatureFlags
  # Flags persist far beyond the cache default TTL (1h). Override with
  # SOLI_FEATURE_TTL (seconds). Default: ~10 years (effectively permanent).
  def self.ttl() -> Int
    raw = getenv("SOLI_FEATURE_TTL")
    return 315360000 if raw.blank?
    return int(raw)
  end

  # ---- read API ------------------------------------------------------------

  # Is `name` on for this user/groups? Returns false for an unknown flag.
  def self.enabled?(name: String, user = null, groups = []) -> Bool
    return false if name.blank?

    # 1. Env override wins outright (kill-switch / CI).
    override = FeatureFlags.env_override(name)
    return override unless override.nil?

    # 2. No stored config -> off by default.
    config = FeatureFlags.get(name)
    return false if config.nil?

    # 3. Global kill-switch.
    return false unless config["on"]

    # 4. Per-user / per-group allowlists always win over the rollout %.
    allow_users = config["users"] || []
    return true if !user.nil? && allow_users.includes?(str(user))

    allow_groups = config["groups"] || []
    for group in groups
      return true if allow_groups.includes?(group)
    end

    # 5. Percentage rollout — deterministic, so a given user is always in or out.
    rollout = config["rollout"]
    if !rollout.nil? && int(rollout) < 100
      return false if user.nil?  # anonymous traffic is never in a partial rollout
      return FeatureFlags.bucket(name + ":" + str(user)) < int(rollout)
    end

    # 6. Plain on, no targeting.
    return true
  end

  # Raw stored config hash for a flag, or null if it was never set.
  # Fail-safe: a cache outage reads as "no config" (flag off) rather than
  # throwing on the request hot path.
  def self.get(name: String) -> Any
    return Cache.get(FeatureFlags.key(name)) rescue null
  end

  # ---- write / admin API ---------------------------------------------------

  def self.enable(name: String)
    config = FeatureFlags.load_or_blank(name)
    config["on"] = true
    FeatureFlags.put(name, config)
  end

  def self.disable(name: String)
    config = FeatureFlags.load_or_blank(name)
    config["on"] = false
    FeatureFlags.put(name, config)
  end

  # Roll out to `percent`% of users (0-100). Stable: the same user keeps the
  # same verdict as you ramp the percentage up.
  def self.set_rollout(name: String, percent: Int)
    config = FeatureFlags.load_or_blank(name)
    config["on"] = true
    config["rollout"] = FeatureFlags.clamp(percent)
    FeatureFlags.put(name, config)
  end

  def self.enable_for(name: String, user)
    config = FeatureFlags.load_or_blank(name)
    config["on"] = true
    users = config["users"] || []
    users.push(str(user)) unless users.includes?(str(user))
    config["users"] = users
    FeatureFlags.put(name, config)
  end

  def self.enable_group(name: String, group: String)
    config = FeatureFlags.load_or_blank(name)
    config["on"] = true
    groups = config["groups"] || []
    groups.push(group) unless groups.includes?(group)
    config["groups"] = groups
    FeatureFlags.put(name, config)
  end

  # Forget a flag entirely (env overrides still apply).
  def self.clear(name: String)
    Cache.delete(FeatureFlags.key(name))
  end

  # ---- internals -----------------------------------------------------------

  def self.key(name: String) -> String
    return "feature:" + name
  end

  def self.blank_config() -> Hash
    return {"on": false, "rollout": 100, "users": [], "groups": []}
  end

  def self.load_or_blank(name: String) -> Hash
    return FeatureFlags.get(name) || FeatureFlags.blank_config()
  end

  def self.put(name: String, config: Hash)
    Cache.set(FeatureFlags.key(name), config, FeatureFlags.ttl())
  end

  def self.clamp(percent: Int) -> Int
    return 0 if percent < 0
    return 100 if percent > 100
    return percent
  end

  # Reads SOLI_FEATURE_<NAME>. Returns true/false for an override, or null
  # when no override is set (so callers can tell "unset" from "false").
  def self.env_override(name: String) -> Any
    raw = getenv("SOLI_FEATURE_" + name.upcase())
    return null if raw.nil?
    return ["1", "true", "on", "yes"].includes?(raw.trim().downcase())
  end

  # Stable 0-99 bucket for a key. Same key -> same bucket on every worker,
  # every request, so rollout membership never flickers.
  def self.bucket(key: String) -> Int
    acc = 0
    for byte in key.bytes()
      acc = (acc * 31 + int(byte)) % 2147483647
    end
    return acc % 100
  end
end
