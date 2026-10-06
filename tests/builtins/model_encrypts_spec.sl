# Encrypted attributes (`encrypts`): AES-256-GCM at rest, plaintext in memory.
# The key comes from SOLI_ENCRYPTION_KEY. Without it every write is refused,
# so the suite splits on whether the runner's environment provides one.

class EncUser < Model
  encrypts(:ssn)
end

# A raw client on the ORM's database, to read what is actually stored.
def raw_db
  db = Solidb(getenv("SOLIDB_HOST") || "http://localhost:6745", db_name())
  username = getenv("SOLIDB_USERNAME")
  db.auth(username, getenv("SOLIDB_PASSWORD")) if username.present?
  db
end

def stored_ssn(key)
  raw_db().query("FOR d IN enc_users FILTER d._key == @k RETURN d.ssn", {"k": key})[0]
end

describe("encrypts") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    EncUser.delete_all()
  end

  context("without SOLI_ENCRYPTION_KEY") do
    before_each() do
      skip("SOLI_ENCRYPTION_KEY is set") if getenv("SOLI_ENCRYPTION_KEY").present?
    end

    test("create reports the missing key and writes nothing") do
      user = EncUser.create({"email": "a@x.co", "ssn": "123-45-6789"})

      assert_eq(user._errors, ["no encryption key: pass one or set SOLI_ENCRYPTION_KEY"])
      assert_null(user._key)
      assert_eq(EncUser.count, 0)
    end

    test("save returns false") do
      user = EncUser.new({"email": "b@x.co", "ssn": "111-11-1111"})

      assert_eq(user.save, false)
      assert_eq(EncUser.count, 0)
    end
  end

  context("with SOLI_ENCRYPTION_KEY") do
    before_each() do
      skip("needs SOLI_ENCRYPTION_KEY in the environment") if getenv("SOLI_ENCRYPTION_KEY").blank?
    end

    test("create and find return plaintext") do
      user = EncUser.create({"email": "a@x.co", "ssn": "123-45-6789"})

      assert_null(user._errors)
      assert_eq(user.ssn, "123-45-6789")
      assert_eq(EncUser.find(user._key).ssn, "123-45-6789")
    end

    test("the stored value is ciphertext that decrypts to the plaintext") do
      user = EncUser.create({"email": "a@x.co", "ssn": "123-45-6789"})
      stored = stored_ssn(user._key)

      assert_ne(stored, "123-45-6789")
      assert_not(stored.contains("123-45-6789"))
      assert_eq(Crypto.decrypt(stored), "123-45-6789")
    end

    test("the same plaintext encrypts differently each time") do
      first = EncUser.create({"email": "a@x.co", "ssn": "123-45-6789"})
      second = EncUser.create({"email": "b@x.co", "ssn": "123-45-6789"})

      assert_ne(stored_ssn(first._key), stored_ssn(second._key))
    end

    test("save updates the ciphertext and still decrypts") do
      user = EncUser.create({"email": "b@x.co", "ssn": "111-11-1111"})
      before = stored_ssn(user._key)

      user.ssn = "222-22-2222"

      assert(user.save)
      assert_eq(user.ssn, "222-22-2222")
      assert_eq(EncUser.find(user._key).ssn, "222-22-2222")
      assert_ne(stored_ssn(user._key), before)
    end

    test("update(hash) re-encrypts too") do
      user = EncUser.create({"email": "b@x.co", "ssn": "111-11-1111"})

      user.update({"ssn": "999-99-9999"})

      assert_eq(user.ssn, "999-99-9999")
      assert_eq(EncUser.find(user._key).ssn, "999-99-9999")
      assert_eq(Crypto.decrypt(stored_ssn(user._key)), "999-99-9999")
    end

    test("a missing or nil field stays nil") do
      absent = EncUser.create({"email": "n@x.co"})
      explicit_nil = EncUser.create({"email": "m@x.co", "ssn": nil})

      assert_null(EncUser.find(absent._key).ssn)
      assert_null(EncUser.find(explicit_nil._key).ssn)
      assert_null(stored_ssn(explicit_nil._key))
    end

    test("plaintext equality does not match stored ciphertext") do
      EncUser.create({"email": "c@x.co", "ssn": "333-33-3333"})

      # Hash where compares the stored ciphertext, which never equals the
      # plaintext (AES-GCM uses a random nonce).
      assert_eq(EncUser.where({"ssn": "333-33-3333"}).count, 0)
      assert_eq(EncUser.where({"email": "c@x.co"}).count, 1)
    end
  end
end
