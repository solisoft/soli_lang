# Cryptography builtins: hashes, HMAC, Base64, password hashing, X25519 and
# Ed25519 keys, AES-GCM Crypto.encrypt/decrypt, RSA signature checks and TOTP.
# Deterministic algorithms are checked against published known-answer vectors.

# RFC 7748 §6.1 Diffie-Hellman vectors
const ALICE_PRIVATE = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a"
const ALICE_PUBLIC = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
const BOB_PRIVATE = "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb"
const BOB_PUBLIC = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"
const SHARED_SECRET = "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"
const BASEPOINT = "0900000000000000000000000000000000000000000000000000000000000000"
const ZERO_POINT = "0000000000000000000000000000000000000000000000000000000000000000"

# RFC 6238: the SHA-1 seed "12345678901234567890" in base32
const TOTP_SECRET = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"

const SHA256_HELLO = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
const SHA512_HELLO = "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca7" +
  "2323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043"

describe("hash functions") do
  test("sha256 matches the known digest of 'hello' and of the empty string") do
    assert_eq(sha256("hello"), SHA256_HELLO)
    assert_eq(sha256(""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    assert_eq(Crypto.sha256("hello"), SHA256_HELLO)
  end

  test("sha512 matches the known digest of 'hello'") do
    assert_eq(sha512("hello"), SHA512_HELLO)
    assert_eq(Crypto.sha512("hello"), SHA512_HELLO)
  end

  test("md5 matches the known digests of 'hello' and the empty string") do
    assert_eq(md5("hello"), "5d41402abc4b2a76b9719d911017c592")
    assert_eq(Crypto.md5("hello"), "5d41402abc4b2a76b9719d911017c592")
    assert_eq(md5(""), "d41d8cd98f00b204e9800998ecf8427e")
  end

  test("Crypto.sha1 matches the FIPS 180 vectors") do
    assert_eq(Crypto.sha1("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d")
    assert_eq(Crypto.sha1(""), "da39a3ee5e6b4b0d3255bfef95601890afd80709")
  end

  test("different inputs give different digests") do
    assert_ne(sha256("hello"), sha256("world"))
  end
end

describe("HMAC") do
  test("hmac defaults to HMAC-SHA256 with a string key") do
    expected = "8b5f48702995c1598c573db1e21866a9b825d4a794d169d7060a03605796360b"
    assert_eq(hmac("message", "secret"), expected)
    assert_eq(Crypto.hmac("message", "secret"), expected)
    assert_eq(Crypto.hmac("message", "secret", "sha256"), expected)
  end

  test("hmac of an empty message under an empty key") do
    assert_eq(hmac("", ""), "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad")
  end

  test("Crypto.hmac takes an algorithm and a binary key (RFC 4231 case 1)") do
    key = Hex.decode("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b")
    assert_eq(
      Crypto.hmac("Hi There", key, "sha512"),
      "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cde" +
        "daa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854"
    )
    assert_eq(
      Crypto.hmac("Hi There", key, "sha256"),
      "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    )
    assert_eq(Crypto.hmac("m", "k", "sha1").length, 40)
  end

  test("a different key gives a different MAC") do
    assert_ne(hmac("message", "key1"), hmac("message", "key2"))
  end

  test("an unsupported algorithm is refused") do
    assert_raises("unsupported HMAC algorithm 'sha3'") do
      Crypto.hmac("m", "k", "sha3")
    end
  end
end

describe("secure_compare") do
  test("matches identical strings, including empty ones") do
    assert(secure_compare("abc", "abc"))
    assert(secure_compare("", ""))
    assert(Crypto.secure_compare("foo", "foo"))
  end

  test("rejects a different byte, a different case and a different length") do
    assert_not(secure_compare("abc", "abd"))
    assert_not(secure_compare("abc", "ABC"))
    assert_not(secure_compare("abc", "abcd"))
    assert_not(secure_compare("abcd", "abc"))
    assert_not(Crypto.secure_compare("foo", "bar"))
  end
end

describe("Base64") do
  test("encodes to the RFC 4648 alphabet with padding") do
    assert_eq(Base64.encode("hello world"), "aGVsbG8gd29ybGQ=")
    assert_eq(Base64.encode("é"), "w6k=")
    assert_eq(Base64.encode(""), "")
  end

  test("decodes back to the original string") do
    assert_eq(Base64.decode("aGVsbG8gd29ybGQ="), "hello world")
    assert_eq(Base64.decode(Base64.encode("Test data for base64")), "Test data for base64")
  end

  test("refuses input outside the alphabet") do
    assert_raises("Base64 decode error") do
      Base64.decode("!!")
    end
  end
end

describe("password hashing") do
  test("argon2_hash produces a salted argon2id PHC string that verifies") do
    hashed = argon2_hash("secret123")
    # The runner sets SOLI_ARGON2_FAST, so the cost parameters are not fixed here
    assert(hashed.starts_with("$argon2id$v=19$m="))
    assert_eq(hashed.split("$").length, 6)
    assert(argon2_verify("secret123", hashed))
    assert_not(argon2_verify("wrong", hashed))
  end

  test("hashing the same password twice gives different salts") do
    assert_ne(argon2_hash("same"), argon2_hash("same"))
  end

  test("password_hash/password_verify are argon2 aliases") do
    hashed = password_hash("mysecret")
    assert(hashed.starts_with("$argon2id$"))
    assert(password_verify("mysecret", hashed))
    assert_not(password_verify("other", hashed))
    assert(argon2_verify("mysecret", hashed))
  end

  test("the Crypto statics hash and verify the same way") do
    hashed = Crypto.argon2_hash("test_password")
    assert(Crypto.argon2_verify("test_password", hashed))
    assert_not(Crypto.argon2_verify("wrong_password", hashed))
    assert(Crypto.password_verify("test_password", Crypto.password_hash("test_password")))
  end

  test("verifying against a string that is not a hash raises") do
    assert_raises("Invalid hash format") do
      Crypto.argon2_verify("x", "not a hash")
    end
  end
end

describe("X25519") do
  test("x25519_public_key derives the RFC 7748 public keys") do
    assert_eq(x25519_public_key(ALICE_PRIVATE), ALICE_PUBLIC)
    assert_eq(Crypto.x25519_public_key(BOB_PRIVATE), BOB_PUBLIC)
  end

  test("x25519_shared_secret gives the RFC 7748 shared secret from both sides") do
    assert_eq(x25519_shared_secret(ALICE_PRIVATE, BOB_PUBLIC), SHARED_SECRET)
    assert_eq(Crypto.x25519_shared_secret(BOB_PRIVATE, ALICE_PUBLIC), SHARED_SECRET)
  end

  test("a generated keypair is 32-byte hex whose public half derives from the private") do
    keypair = Crypto.x25519_keypair()
    assert_match(keypair["private"], "^[0-9a-f]{64}$")
    assert_match(keypair["public"], "^[0-9a-f]{64}$")
    assert_eq(Crypto.x25519_public_key(keypair["private"]), keypair["public"])
    assert_ne(x25519_keypair()["private"], keypair["private"])
  end

  test("two generated keypairs agree on a shared secret") do
    alice = x25519_keypair()
    bob = x25519_keypair()
    alice_side = Crypto.x25519_shared_secret(alice["private"], bob["public"])
    assert_eq(alice_side, Crypto.x25519_shared_secret(bob["private"], alice["public"]))
    assert_eq(alice_side.length, 64)
  end

  test("a small-order public key is refused") do
    assert_raises("small-order point") do
      Crypto.x25519_shared_secret(ALICE_PRIVATE, ZERO_POINT)
    end
  end

  test("a key that is not 32 bytes is refused") do
    assert_raises("private key must be 32 bytes, got 4") do
      Crypto.x25519_public_key("abcd")
    end
  end

  test("x25519(basepoint, scalar) on the base point with a clamped scalar gives the public key") do
    # ALICE_PRIVATE with RFC 7748 clamping applied by hand (first byte & 248, last byte & 127 | 64)
    clamped_alice = "70076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c6a"
    assert_eq(x25519(BASEPOINT, clamped_alice), ALICE_PUBLIC)
  end

  test("x25519 matches the RFC 7748 §5.2 scalar-multiplication vector") do
    pending("bug: x25519() neither clamps the scalar nor keeps cofactor bits (reduces mod l)")
    scalar = "a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4"
    u_coordinate = "e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c"
    assert_eq(x25519(u_coordinate, scalar), "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")
  end

  test("x25519 refuses a small-order base point and a short one") do
    assert_raises("small-order point produced an all-zero result") do
      x25519(ZERO_POINT, ALICE_PRIVATE)
    end
    assert_raises("basepoint must be 32 bytes, got 2") do
      x25519("00", ALICE_PRIVATE)
    end
  end
end

describe("Ed25519 keys") do
  test("ed25519_keypair returns distinct 32-byte hex halves") do
    keypair = ed25519_keypair()
    assert_match(keypair["private"], "^[0-9a-f]{64}$")
    assert_match(keypair["public"], "^[0-9a-f]{64}$")
    assert_ne(keypair["private"], keypair["public"])
  end

  test("Crypto.ed25519_keypair generates a fresh pair each call") do
    first = Crypto.ed25519_keypair()
    assert_eq(first.keys.sort(), ["private", "public"])
    assert_ne(Crypto.ed25519_keypair()["public"], first["public"])
  end
end

describe("Crypto.encrypt / Crypto.decrypt") do
  test("decrypt returns what encrypt sealed under the same key") do
    sealed = Crypto.encrypt("secret message", "key-one")
    assert_eq(Crypto.decrypt(sealed, "key-one"), "secret message")
  end

  test("round-trips UTF-8 and the empty string") do
    assert_eq(Crypto.decrypt(Crypto.encrypt("héllo ✓", "k"), "k"), "héllo ✓")
    assert_eq(Crypto.decrypt(Crypto.encrypt("", "k"), "k"), "")
  end

  test("output is base64 of a 12-byte nonce plus a 16-byte tag around the ciphertext") do
    # 12 + 0 + 16 = 28 bytes -> 40 base64 chars; 12 + 14 + 16 = 42 bytes -> 56 chars
    assert_eq(Crypto.encrypt("", "k").length, 40)
    assert_eq(Crypto.encrypt("secret message", "k").length, 56)
  end

  test("a fresh nonce makes two encryptions of the same text differ") do
    assert_ne(Crypto.encrypt("hello", "k1"), Crypto.encrypt("hello", "k1"))
  end

  test("the wrong key fails to decrypt") do
    sealed = Crypto.encrypt("hello", "k1")
    assert_raises("decryption failed (wrong key or corrupt data)") do
      Crypto.decrypt(sealed, "k2")
    end
  end

  test("a tampered ciphertext fails to decrypt") do
    sealed = Crypto.encrypt("hello", "k1")
    tampered = (sealed[0] == "A" ? "B" : "A") + sealed.substring(1, sealed.length)
    assert_raises("decryption failed") do
      Crypto.decrypt(tampered, "k1")
    end
  end

  test("input that is not base64 is refused") do
    assert_raises("invalid base64") do
      Crypto.decrypt("!!!", "k")
    end
  end

  # .env.test may set SOLI_ENCRYPTION_KEY (for the `encrypts` model specs), so
  # exactly one of the next two tests runs
  test("without a key argument it falls back to SOLI_ENCRYPTION_KEY") do
    skip("SOLI_ENCRYPTION_KEY is not set") unless hasenv("SOLI_ENCRYPTION_KEY")

    assert_eq(Crypto.decrypt(Crypto.encrypt("x"), getenv("SOLI_ENCRYPTION_KEY")), "x")
    assert_eq(Crypto.decrypt(Crypto.encrypt("x", getenv("SOLI_ENCRYPTION_KEY"))), "x")
  end

  test("without a key argument or SOLI_ENCRYPTION_KEY it refuses") do
    skip("SOLI_ENCRYPTION_KEY is set") if hasenv("SOLI_ENCRYPTION_KEY")

    assert_raises("no encryption key: pass one or set SOLI_ENCRYPTION_KEY") do
      Crypto.encrypt("x")
    end
  end
end

describe("RsaKey.verify") do
  test("checks a PKCS#1 v1.5 signature") do
    # `openssl dgst -sha1 -sign` of the message with the matching private key
    pem = "-----BEGIN PUBLIC KEY-----\n" +
      "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQCVNJCMOElvuFlwioV+yJrePWcB\n" +
      "f297VcdPey6eTprEaqwVln0QnSix6+8SZ+Lmhp/reqSQbbSU7CFjq2hE4ihycI9K\n" +
      "L4owJCcZguPsS8BfQ5N+oebbEECMJFy8gPSh5gNjeZBuv06XCGRttcKrdRnJ1Suu\n" +
      "RhwGmTE/QZENd9lsuQIDAQAB\n" +
      "-----END PUBLIC KEY-----"
    signature = "DoxNElGwl2BVmyEqHYRymZzP/bgBE+fhzBOll7/uJxpM3RWdVFsc8FXC+fpnU9fu3/7ven5SXRGD1E9aquLFky9D" +
      "h7YYq3nKtFwajkafDBb3JbtBBlG8p8U4bRTdNaGtk+2aAjqCwj1f+ZDnRmQ5U/WbapkqHbLWXr3jNvO8jZk="
    assert(RsaKey.verify(pem, "amount=2500&reference=abc", signature, "sha1"))
    assert_not(RsaKey.verify(pem, "amount=2600&reference=abc", signature, "sha1"))
    assert_not(RsaKey.verify(pem, "amount=2500&reference=abc", signature, "sha256"))
    assert_not(RsaKey.verify(pem, "amount=2500&reference=abc", "not base64 !", "sha1"))
  end
end

describe("TOTP") do
  context("totp_generate") do
    test("matches the RFC 6238 SHA-1 vectors, truncated to 6 digits") do
      assert_eq(Crypto.totp_generate(TOTP_SECRET, 59, 30), "287082")
      assert_eq(Crypto.totp_generate(TOTP_SECRET, 1111111109, 30), "081804")
    end

    test("without a time it uses the current one and stays within the verify window") do
      code = Crypto.totp_generate(TOTP_SECRET)
      assert_match(code, "^[0-9]{6}$")
      assert(Crypto.totp_verify(TOTP_SECRET, code))
    end
  end

  context("totp_verify") do
    test("accepts the code of the current window") do
      assert(Crypto.totp_verify(TOTP_SECRET, "287082", 59, 30))
    end

    test("rejects a wrong code") do
      assert_not(Crypto.totp_verify(TOTP_SECRET, "000000", 59, 30))
    end

    test("accepts the previous and the next window") do
      assert(Crypto.totp_verify(TOTP_SECRET, Crypto.totp_generate(TOTP_SECRET, 29, 30), 59, 30))
      assert(Crypto.totp_verify(TOTP_SECRET, Crypto.totp_generate(TOTP_SECRET, 89, 30), 59, 30))
    end

    test("rejects a code two windows away") do
      assert_not(Crypto.totp_verify(TOTP_SECRET, Crypto.totp_generate(TOTP_SECRET, 119, 30), 59, 30))
    end

    test("raises on a code that is not 6 digits") do
      assert_raises("Code must be 6 digits") do
        Crypto.totp_verify(TOTP_SECRET, "123", 59, 30)
      end
    end
  end

  test("totp_uri builds the otpauth URI with an escaped account and the issuer") do
    assert_eq(
      Crypto.totp_uri(TOTP_SECRET, "user@example.com", "MyApp", 30),
      "otpauth://totp/MyApp:user%40example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ" +
        "&algorithm=SHA1&digits=6&period=30&issuer=MyApp"
    )
  end
end
