# Passkeys driven by a real browser and Chrome's virtual authenticator.
#
# No database: the challenge and the one registered passkey live in the
# session, enough to prove the browser half (navigator.credentials) and the
# server half (WebAuthn.*) agree. The full flow with accounts is the
# generated app's (`soli generate auth --passkeys`, passkeys_generator_test).
#
# WebAuthn refuses an IP address as the relying party, so the spec drives this
# page on http://localhost:<port>; the RP ID and origin follow the Host header.
class PasskeysController < Controller
  # GET /passkeys
  def index(req)
    render("passkeys/index", {"title": "Passkeys"})
  end

  # POST /passkeys/register/options
  def register_options(req)
    result = WebAuthn.registration_options({
      "rp_id": @_rp_id(req),
      "rp_name": "Browser fixture",
      "user_id": "user-1",
      "user_name": "alice@example.com"
    })
    session_set("passkey_challenge", result["challenge"])
    render_json(result["options"])
  end

  # POST /passkeys/register
  def register(req)
    expected = @_expected(req)
    try
      record = WebAuthn.verify_registration(req["json"], expected)
    catch error
      return render_json({"error": error.to_s}, 422)
    end
    session_set("passkey", record)
    render_json({
      "ok": true,
      "algorithm": record["algorithm"],
      "attestation_type": record["attestation_type"],
      "user_verified": record["user_verified"]
    })
  end

  # POST /passkeys/login/options
  def login_options(req)
    result = WebAuthn.authentication_options({"rp_id": @_rp_id(req)})
    session_set("passkey_challenge", result["challenge"])
    render_json(result["options"])
  end

  # POST /passkeys/login
  def login(req)
    passkey = session_get("passkey")
    return render_json({"error": "no passkey registered"}, 401) if passkey.nil?

    expected = @_expected(req)
    expected["public_key"] = passkey["public_key"]
    expected["sign_count"] = passkey["sign_count"]
    expected["credential_id"] = passkey["credential_id"]
    try
      result = WebAuthn.verify_authentication(req["json"], expected)
    catch error
      return render_json({"error": error.to_s}, 401)
    end
    render_json({"ok": true, "user_handle": result["user_handle"], "sign_count": result["sign_count"]})
  end

  private

  def _rp_id(req)
    req["headers"]["host"].to_s.split(":")[0]
  end

  # The challenge is single use: taken out of the session before verifying.
  def _expected(req)
    challenge = session_get("passkey_challenge")
    session_delete("passkey_challenge")
    {"challenge": challenge.to_s, "origins": ["http://" + req["headers"]["host"].to_s], "rp_id": @_rp_id(req)}
  end
end
