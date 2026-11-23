//! Templates for `soli generate devices`.

pub const DEVICE_MODEL: &str = r#"# Device — push / native shell registration targets.
#
# Shells POST { platform, token } (or a web PushSubscription) after login.
# Pass Device.push_targets_for(user_id) into Push.deliver(..., { "targets": ... }).

# The model `soli generate devices` wrote, by name: a password reset of the
# auth scaffold removes an account's push devices, and looks here to know
# which model that is — not any model that happens to be called Device.
const PUSH_DEVICES_MODEL = "Device"

class Device < Model
  validates("platform", { "presence": true })
  validates("token", { "presence": true })

  # Upsert by token so a reinstall or token rotate does not stack rows.
  static def register(user_id, platform, token, subscription = null) -> Any {
    let existing = Device.first_by("token", token)
    if existing {
      existing.user_id = user_id
      existing.platform = platform
      existing.subscription = subscription
      existing.save()
      return existing
    }
    Device.create({
      "user_id": user_id,
      "platform": platform,
      "token": token,
      "subscription": subscription
    })
  }

  static def push_targets_for(user_id) -> Array {
    Device.where("doc.user_id == @uid", { "uid": user_id }).all().map(fn(d) {
      if d.platform == "web" && d.subscription {
        { "platform": "web", "subscription": d.subscription }
      } else {
        { "platform": d.platform, "token": d.token }
      }
    })
  }

  static def prune_tokens(tokens) -> Int {
    let n = 0
    for token in tokens {
      let row = Device.first_by("token", token)
      if row {
        row.delete()
        n = n + 1
      }
    }
    n
  }
end
"#;

pub const DEVICES_CONTROLLER: &str = r#"# DevicesController — register push tokens from native shells and browsers.
#
# POST /devices  { platform, token?, subscription? }
# DELETE /devices/:id

def create(req)
  # The signed-in user as the middleware left it, not the session's
  # `user_id`: the session holds that id as soon as the password passed,
  # before a second factor and whatever a deactivation says — the middleware
  # that enforce those clear it, and this action must answer to them. Read
  # from the request, where the auth scaffold's middleware (or the app's
  # own) puts it: `current_user()` is that scaffold's helper, and an app
  # without it has none.
  let user = req["current_user"] rescue nil
  let user_id = user.nil? ? nil : user._key
  if user_id.nil?
    return { "status": 401, "body": json_stringify({ "error": "login required" }) }
  end
  # Not while an administrator is acting as the user: the device registered
  # would be the administrator's, and would go on receiving this account's
  # notifications after the impersonation ended.
  unless session_get("impersonator_id").to_s().blank?
    return { "status": 403, "body": json_stringify({ "error": "not while acting as another user" }) }
  end

  let params = req["json"] || {}
  let platform = (params["platform"] || "").to_s().downcase()
  let token = params["token"]
  let subscription = params["subscription"]

  unless ["ios", "android", "web", "macos", "apple", "fcm"].includes?(platform)
    return { "status": 422, "body": json_stringify({ "error": "invalid platform" }) }
  end

  if platform == "web"
    if subscription.nil?
      return { "status": 422, "body": json_stringify({ "error": "subscription required for web" }) }
    end
    # The endpoint is what names a web subscription — and what a failed
    # delivery reports, for `Device.prune_tokens` to find the row by.
    let endpoint = subscription["endpoint"] rescue nil
    token = token || endpoint || json_stringify(subscription)
  elsif token.nil? || token.to_s().blank?
    return { "status": 422, "body": json_stringify({ "error": "token required" }) }
  end

  let device = Device.register(user_id, platform, token.to_s(), subscription)
  # Not from a session that ended while this was written (a password reset):
  # the device would go on receiving the account's notifications.
  # (Where the session carries the account's session version — the auth
  # scaffold's does; an app with its own sign-in has nothing to compare.)
  let stamped = session_get("session_version")
  unless stamped.nil?
    let fresh = nil
    try
      fresh = User.find_by("_key", user_id)
    catch error
      # The question could not be asked: the device is not kept on the
      # strength of an answer nobody gave.
      device.delete() rescue nil
      throw error
    end
    if fresh.nil? || (fresh.session_version ?? 0).to_i() != (user.session_version ?? 0).to_i()
      device.delete()
      return { "status": 401, "body": json_stringify({ "error": "login required" }) }
    end
  end
  {
    "status": 201,
    "headers": { "Content-Type": "application/json" },
    "body": json_stringify({
      "id": device._key || device.id,
      "platform": device.platform,
      "token": device.token
    })
  }
end

def destroy(req)
  # The signed-in user as the middleware left it, not the session's
  # `user_id`: the session holds that id as soon as the password passed,
  # before a second factor and whatever a deactivation says — the middleware
  # that enforce those clear it, and this action must answer to them. Read
  # from the request, where the auth scaffold's middleware (or the app's
  # own) puts it: `current_user()` is that scaffold's helper, and an app
  # without it has none.
  let user = req["current_user"] rescue nil
  let user_id = user.nil? ? nil : user._key
  if user_id.nil?
    return { "status": 401, "body": json_stringify({ "error": "login required" }) }
  end
  unless session_get("impersonator_id").to_s().blank?
    return { "status": 403, "body": json_stringify({ "error": "not while acting as another user" }) }
  end

  let id = req["params"]["id"]
  let device = Device.find(id) rescue null
  if device.nil? || device.user_id != user_id
    return { "status": 404, "body": json_stringify({ "error": "not found" }) }
  end
  device.delete()
  { "status": 204, "body": "" }
end
"#;

pub const DEVICES_HELPER: &str = r#"# Helpers for closed-app push with prune.

def deliver_to_user(user_id, payload, options = {})
  let targets = Device.push_targets_for(user_id)
  let opts = options.merge({ "targets": targets })
  let result = Push.deliver("user:#{str(user_id)}", payload, opts)
  Device.prune_tokens(result["prune"] || [])
  result
end
"#;

pub const ROUTES_MARKER: &str = "# --- soli generate devices ---";

pub fn routes_snippet() -> String {
    format!(
        r#"
{marker}
# No `skip_csrf` here: the generated shells send `Origin` with their token
# POST, so the same-origin gate lets them through — and keeps out a page on
# another site registering its own device (and so receiving this user's
# notifications).
post("/devices", "devices#create");
delete("/devices/:id", "devices#destroy");
# --- end soli generate devices ---
"#,
        marker = ROUTES_MARKER
    )
}

pub fn devices_migration() -> String {
    r#"# Migration: create_devices
# Generated by: soli generate devices
#
# begin/rescue so a re-run (or a collection SoliDB already auto-created on
# first model access) is not a hard failure.

def up(db)
  begin
    db.create_collection("devices")
  rescue
    print("  skip devices collection (already exists)")
  end

  begin
    db.create_index("devices", "idx_devices_token", ["token"], { "unique": true })
  rescue
    print("  skip idx_devices_token (already exists)")
  end

  begin
    db.create_index("devices", "idx_devices_user_id", ["user_id"], {})
  rescue
    print("  skip idx_devices_user_id (already exists)")
  end
end

def down(db)
  begin
    db.list_indexes("devices").each do |idx|
      db.drop_index("devices", idx["name"])
    end
  rescue
    print("  skip dropping devices indexes (collection is gone)")
  end

  begin
    db.drop_collection("devices")
  rescue
    print("  skip dropping devices collection (already gone)")
  end
end
"#
    .to_string()
}
