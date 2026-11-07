# WebAuthn (server-side passkeys) through the class dispatch, on both engines.
# The Rust unit tests in src/interpreter/builtins/webauthn.rs cover one
# negative case per verification step; this spec goes through `WebAuthn.*` the
# way a controller does, with the OpenSSL-signed vectors in
# tests/fixtures/webauthn/vectors.json (see generate.py next to them).

const VECTORS_PATH = "tests/fixtures/webauthn/vectors.json"
const RP_ID = "example.com"
const ORIGIN = "https://example.com"

def load_vectors
  JSON.parse(slurp(VECTORS_PATH))
end

def registration_expected(vector)
  {"challenge": vector["registration_challenge"], "origins": [ORIGIN], "rp_id": RP_ID}
end

def authentication_expected(vector)
  {
    "challenge": vector["authentication_challenge"],
    "origins": ORIGIN,
    "rp_id": RP_ID,
    "public_key": vector["public_key"],
    "sign_count": 0
  }
end

describe("WebAuthn") do
  context("registration_options") do
    test("builds PublicKeyCredentialCreationOptionsJSON and returns the challenge") do
      result = WebAuthn.registration_options({
        "rp_id": RP_ID,
        "rp_name": "Example",
        "user_id": "user-1",
        "user_name": "alice@example.com",
        "exclude_credentials": ["AAEC"]
      })
      options = result["options"]
      assert_eq(options["challenge"], result["challenge"])
      assert_match(result["challenge"], "^[A-Za-z0-9_-]{43}$")
      assert_eq(options["rp"], {"id": RP_ID, "name": "Example"})
      assert_eq(options["user"]["id"], "dXNlci0x")
      assert_eq(options["user"]["displayName"], "alice@example.com")
      assert_eq(options["attestation"], "none")
      assert_eq(options["pubKeyCredParams"].map { |param| param["alg"] }, [-7, -257])
      assert_eq(options["excludeCredentials"], [{"id": "AAEC", "type": "public-key"}])
      assert_eq(options["authenticatorSelection"]["residentKey"], "required")
    end

    test("never repeats a challenge") do
      opts = {"rp_id": RP_ID, "user_id": "u", "user_name": "n"}
      assert_ne(WebAuthn.registration_options(opts)["challenge"], WebAuthn.registration_options(opts)["challenge"])
    end

    test("refuses an unsupported algorithm") do
      assert_raises("WebAuthn.registration_options(): algorithm -8 is not supported") do
        WebAuthn.registration_options({"rp_id": RP_ID, "user_id": "u", "user_name": "n", "algorithms": [-8]})
      end
    end
  end

  context("authentication_options") do
    test("builds PublicKeyCredentialRequestOptionsJSON") do
      result = WebAuthn.authentication_options({"rp_id": RP_ID})
      assert_eq(result["options"]["rpId"], RP_ID)
      assert_eq(result["options"]["allowCredentials"], [])
      assert_eq(result["options"]["userVerification"], "preferred")
      assert_eq(result["options"]["challenge"], result["challenge"])
    end
  end

  context("verify_registration") do
    test("accepts an ES256 and an RS256 credential") do
      vectors = load_vectors()
      [["es256", -7], ["rs256", -257]].each do |pair|
        vector = vectors[pair[0]]
        record = WebAuthn.verify_registration(vector["registration"], registration_expected(vector))
        assert_eq(record["algorithm"], pair[1])
        assert_eq(record["credential_id"], vector["credential_id"])
        assert_eq(record["public_key"], vector["public_key"])
        assert_eq(record["sign_count"], 0)
        assert_eq(record["transports"], ["internal", "hybrid"])
        assert_eq(record["attestation_format"], "none")
        assert(record["user_verified"])
        assert(record["backup_eligible"])
        assert(!record["backed_up"])
      end
    end

    test("raises on another origin, naming both") do
      vector = load_vectors()["es256"]
      expected = registration_expected(vector)
      expected["origins"] = ["https://other.example"]
      assert_raises("WebAuthn.verify_registration(): origin mismatch (expected https://other.example, got https://example.com)") do
        WebAuthn.verify_registration(vector["registration"], expected)
      end
    end

    test("raises on another challenge") do
      vector = load_vectors()["es256"]
      expected = registration_expected(vector)
      expected["challenge"] = vector["authentication_challenge"]
      assert_raises("WebAuthn.verify_registration(): challenge mismatch") do
        WebAuthn.verify_registration(vector["registration"], expected)
      end
    end

    test("raises on another RP ID") do
      vector = load_vectors()["es256"]
      expected = registration_expected(vector)
      expected["rp_id"] = "other.example"
      assert_raises("WebAuthn.verify_registration(): rpIdHash mismatch") do
        WebAuthn.verify_registration(vector["registration"], expected)
      end
    end

    test("raises on an assertion passed as a registration") do
      vector = load_vectors()["es256"]
      assert_raises("WebAuthn.verify_registration(): clientData type mismatch (expected webauthn.create, got webauthn.get)") do
        WebAuthn.verify_registration(vector["authentication"], registration_expected(vector))
      end
    end

    test("is rescuable to nil") do
      vector = load_vectors()["es256"]
      expected = registration_expected(vector)
      expected["rp_id"] = "other.example"
      record = WebAuthn.verify_registration(vector["registration"], expected) rescue nil
      assert(record.nil?)
    end
  end

  context("verify_authentication") do
    test("verifies the signature with the stored key, on both algorithms") do
      vectors = load_vectors()
      ["es256", "rs256"].each do |name|
        vector = vectors[name]
        result = WebAuthn.verify_authentication(vector["authentication"], authentication_expected(vector))
        assert_eq(result["credential_id"], vector["credential_id"])
        assert_eq(result["user_handle"], "user-1")
        assert_eq(result["sign_count"], 1)
        assert(result["user_verified"])
        assert(!result["counter_regressed"])
      end
    end

    test("flags a counter that did not move forward") do
      vector = load_vectors()["es256"]
      expected = authentication_expected(vector)
      expected["sign_count"] = 1
      result = WebAuthn.verify_authentication(vector["authentication"], expected)
      assert(result["counter_regressed"])
    end

    test("raises when the stored key is another credential's") do
      vectors = load_vectors()
      vector = vectors["es256"]
      expected = authentication_expected(vector)
      expected["public_key"] = vectors["rs256"]["public_key"]
      assert_raises("WebAuthn.verify_authentication(): signature is invalid") do
        WebAuthn.verify_authentication(vector["authentication"], expected)
      end
    end

    test("raises when the credential id is not the stored one") do
      vector = load_vectors()["es256"]
      expected = authentication_expected(vector)
      expected["credential_id"] = load_vectors()["rs256"]["credential_id"]
      assert_raises("WebAuthn.verify_authentication(): credential id does not match the stored credential") do
        WebAuthn.verify_authentication(vector["authentication"], expected)
      end
    end

    test("passes require_user_verification when the assertion carries UV") do
      vector = load_vectors()["es256"]
      expected = authentication_expected(vector)
      expected["require_user_verification"] = true
      assert(WebAuthn.verify_authentication(vector["authentication"], expected)["user_verified"])
    end

    test("raises on a registration response replayed as an assertion") do
      vector = load_vectors()["es256"]
      registration = vector["registration"]
      registration["response"]["authenticatorData"] = vector["authentication"]["response"]["authenticatorData"]
      registration["response"]["signature"] = vector["authentication"]["response"]["signature"]
      assert_raises("WebAuthn.verify_authentication(): clientData type mismatch (expected webauthn.get, got webauthn.create)") do
        WebAuthn.verify_authentication(registration, authentication_expected(vector))
      end
    end

    test("requires the expected hash") do
      assert_raises("WebAuthn.verify_authentication(): expected must be a Hash, got string") do
        WebAuthn.verify_authentication({}, "nope")
      end
    end
  end
end
