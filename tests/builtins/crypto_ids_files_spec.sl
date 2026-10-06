# Offline builtins: Crypto.ledger_hash / merkle_root, Hex, RsaKey.public_from_pem,
# X509.spki_pin, UUID / ULID / NanoID, File and Trusted globbing, file write helpers,
# Logger.warn, Factory lists and sequences, expect().to_match, string globals and
# the comparison assertions' failure paths.

const RSA_KEY_PEM = "-----BEGIN PRIVATE KEY-----\n" +
  "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCca/ZMr23wsIM9\n" +
  "LY5nPcV4ViQxdQ30Ca65FZMCZXQYQy+edBMeXeVF34DzVKSZUkyAKZ93U6ShZVkb\n" +
  "kwvqFJP1jYrzMU+ov2eempYg0N9WDG02NaMw65ZVaKHbR5N4axHHxGLpdZbwVPPl\n" +
  "sAIm7A1YMk4fF4gtneED4jjXtCVYwzTkB2HWegAYqpSkuzeyp4F/6LFCPOQ5VDqg\n" +
  "ZRNSXCZVQC42Dmt0QMNICqyKjcfTB/IAS1DtRYwqBfADNPS+OGnARd5iU60gBCCL\n" +
  "BD38G3ZDAI/P3HsonXpfo7w+6sMn2B8V8VPvd9O32p5PB+NrmIcIJpocipIngM0L\n" +
  "OPLaC8eNAgMBAAECggEAKAb+i4gWz5EzvDuEpcmqVw1gDKHiFLFHm0g4itPwXecP\n" +
  "b/JPFCW97l/vzRS7XBqxxdgg3PWz+rMHFuXNljR22k7CoFHdixaTywPO6A3bINdk\n" +
  "OQuHu5SFr0xrosPRqm5nqeGI2CoFmnF6yit8mX4tOgUBdbZdXCL6+jXxCs2oAurm\n" +
  "e4hfNmrxoG599cpl58p8Yg+JPrsiPWQxeRU7tVNRGUbovT3jYYDsOpcI4qlNZIcd\n" +
  "fcZHIkI1/5J5B08cWcyc88xIEFUrHFoe01TMXTrt3GosmF+1VSyZGv1P2y7CX7j6\n" +
  "UqxYyom0nK7eWXI+XbpqIAmLCykMIqofs3lyRyXIJwKBgQDSLtGpcw4XbTVhMjF8\n" +
  "xjQk2TaI5cky+hr9y19mfHdY/xld9k0aiT8CbNTUhlyFf8ZapwussBhdTBiOKYtQ\n" +
  "PWyzIyjq99pedep9Ze4RuhFNCMfdogZcj6RT1Bi1tNtedKaCIJCNo/RdmH3t8798\n" +
  "2lW7KMDC6bQd5pE+bmyvT8fnowKBgQC+hQVRtGBkDNqL2YpC7lZm2O8XKUFbV+Z7\n" +
  "JZhEZGYaxiCN+1fywB+EL41xGuVmOxlxuOrQ33ugHtlt8YEarX3vUQlxxPXsU+Fm\n" +
  "ECgTXEiAYn9TmNTB/q075Ed5HC5+poyRfgTpNSKeXMvDcMY9Xi4nIIAC1IefJRDU\n" +
  "OAwc8P1HDwKBgQCBXhPqakjYHn3mj1BqbkyWCaRJarYGTG7km5Lir+V9v7ZLYVhf\n" +
  "5u4Dfh0ZmoHEIbti/MJwzgqREk9i4StAfi4zrIZ46Ylc7tMfz+dSveX8NlVek2W6\n" +
  "/yaz+i4jWWhUoRQDsCuJIss7+Ko6Ffdcz75I7nKHBfW5Gbt4Y9s9pKt0ZQKBgEwn\n" +
  "9iFb3fAAZ1fhxG/Ov8Dq1F/IwPRXZa0yMPSdwWbQbfDzWIuTmsWHEJ32p14/H4Oi\n" +
  "7FJEEzHFQxq8n+PfF+kS1pigp8EpIn9e0/YxPFX9iXIMNHe7atn2/U7/IeLEhood\n" +
  "+q6R692rsFPWf5fGTuKbDjCTbgcClQCPyt/CwSunAoGBAM/VB/icg3q/pyRERic2\n" +
  "XKTomPi/5gruQgSlrHw93k1XObk+jOt1Xba6uDhUiXRxWwRz7lqpZ2l6NflbngNp\n" +
  "Y/0tUksNJ0PRhV3NLdVKsZdE/1fl4kbxNE5VMCUn1IHBFFHSaM4Rk5vfAYBLwAyW\n" +
  "p1uYKl5VM/9SLSt39wJM9nqx\n" +
  "-----END PRIVATE KEY-----\n"

# SPKI public half of RSA_KEY_PEM
const RSA_PUBLIC_PEM = "-----BEGIN PUBLIC KEY-----\n" +
  "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAnGv2TK9t8LCDPS2OZz3F\n" +
  "eFYkMXUN9AmuuRWTAmV0GEMvnnQTHl3lRd+A81SkmVJMgCmfd1OkoWVZG5ML6hST\n" +
  "9Y2K8zFPqL9nnpqWINDfVgxtNjWjMOuWVWih20eTeGsRx8Ri6XWW8FTz5bACJuwN\n" +
  "WDJOHxeILZ3hA+I417QlWMM05Adh1noAGKqUpLs3sqeBf+ixQjzkOVQ6oGUTUlwm\n" +
  "VUAuNg5rdEDDSAqsio3H0wfyAEtQ7UWMKgXwAzT0vjhpwEXeYlOtIAQgiwQ9/Bt2\n" +
  "QwCPz9x7KJ16X6O8PurDJ9gfFfFT73fTt9qeTwfja5iHCCaaHIqSJ4DNCzjy2gvH\n" +
  "jQIDAQAB\n" +
  "-----END PUBLIC KEY-----\n"

# Self-signed certificate for sp.example.com (a different key from RSA_KEY_PEM)
const CERT_PEM_A = "-----BEGIN CERTIFICATE-----\n" +
  "MIICqzCCAZOgAwIBAgIBATANBgkqhkiG9w0BAQsFADAZMRcwFQYDVQQDDA5zcC5l\n" +
  "eGFtcGxlLmNvbTAeFw0yMDAxMDEwMDAwMDBaFw0zNTAxMDEwMDAwMDBaMBkxFzAV\n" +
  "BgNVBAMMDnNwLmV4YW1wbGUuY29tMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIB\n" +
  "CgKCAQEAtAJl23hnRhGvpJ5QQ4haLfWK3mjd8Slf8fnhUWD/Qzoo3I1bsvcr8HAj\n" +
  "kpaIKne5aLU1jp/migvcdgDx33JAck23I7pz9Yq47sKA1/KP+TFxmzOAA08M34xZ\n" +
  "O4BPD1VWDBNZyfwtX5FA8eQiuBI0UIkLksulLb9wPjqd28vDAFVldGvlXMQRVZEW\n" +
  "n0Y7xqC2gGCNAM2y7N3oDNPvAIuIstFNGExh+bP/J7PDZeTIhR2q1FsGoeZwGMO1\n" +
  "U8EzJQj+D2ZahfF2aKXtHOo511EQsmXEx/DhpoKrgvClv9/jZpYxxSmUocsYCiEQ\n" +
  "gRi/0YadfTJzLAUGGgMNRGgfzhfiqwIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQCp\n" +
  "03SxaAvxD1c+pMg4Q3YTPFUe5eFRvYxaXG8BdDCH9P+uD+TVWstPr5Rx7pWDGsuV\n" +
  "9QlyNmA3bFea4Ps8n7CiEuiJeDzbtTznOBHbF5/AUj7fhNHu9Su0Ka4Fg5QCuRGZ\n" +
  "B6Z6fkJDkZ0NVRJXwqXgOByvm7i0VE0mtFaf1kyqApPV2IohF/CxfqsMz/dySWPl\n" +
  "ODWt3qmRBU3Wk5wUtD+71Opmb+qfXZoqFuKoY1MHSf14rXcV/tETLyhXp8oaA/OM\n" +
  "b9onERfWbd8xx//ct4TFLUo64uvsyXtFhnAcjVec/qOMUzZ/OmzVW8caCThFIhnt\n" +
  "2StVAKOJevnZUkGGHuyN\n" +
  "-----END CERTIFICATE-----\n"

# Raw DER (base64) certificate carrying the RSA_KEY_PEM public key
const CERT_DER_B64_B = "MIICqzCCAZOgAwIBAgIBATANBgkqhkiG9w0BAQsFADAZMRcwFQYDVQQDDA5zcC5leGFtcGxlLmNvbTAeFw0yMDAxMD" +
  "EwMDAwMDBaFw0zNTAxMDEwMDAwMDBaMBkxFzAVBgNVBAMMDnNwLmV4YW1wbGUuY29tMIIBIjANBgkqhkiG9w0BAQEF" +
  "AAOCAQ8AMIIBCgKCAQEAnGv2TK9t8LCDPS2OZz3FeFYkMXUN9AmuuRWTAmV0GEMvnnQTHl3lRd+A81SkmVJMgCmfd1" +
  "OkoWVZG5ML6hST9Y2K8zFPqL9nnpqWINDfVgxtNjWjMOuWVWih20eTeGsRx8Ri6XWW8FTz5bACJuwNWDJOHxeILZ3h" +
  "A+I417QlWMM05Adh1noAGKqUpLs3sqeBf+ixQjzkOVQ6oGUTUlwmVUAuNg5rdEDDSAqsio3H0wfyAEtQ7UWMKgXwAz" +
  "T0vjhpwEXeYlOtIAQgiwQ9/Bt2QwCPz9x7KJ16X6O8PurDJ9gfFfFT73fTt9qeTwfja5iHCCaaHIqSJ4DNCzjy2gvH" +
  "jQIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQBUHvn9HJEMkw4NZ95aSbmNAF+uMoCOCuj9NotJrmbUFdjLpfSvWpx4lS" +
  "I6ZoUdXa7QyN/Wi7gBJgSl1ex6axSAD3Al5fEZqBuEqsMMncRGf2y463MGDgUYutzWPP4KDomdDVMzogpz0EuG2HGr" +
  "S09YWy34KiBJ9bP48TaWn5iosauGFzv+zE2fTF/YK6sxbOTOgU8mTteXwbHwYZSBrCLUj/dcAbrNCrE8IezxmUJR6W" +
  "/byABg+tCSW1cVhjH9BrBp6uFgXESMY+tnUJeZcMTf+24hadsr0X4ivV1BXE20O2lZblesg6oxjvjnSsBNH2BDdZYM" +
  "cX+4eEI3MN83Tesw"


def fresh_tmp_dir
  "/tmp/soli_cif_spec_" + uuid_v4()
end

# A comparison assertion that fails cannot be observed with assert_raises: an
# assertion failure inside its block is re-raised as the test's own failure.
# Catch it by hand and return its message (nil when it passed).
def assertion_failure(check)
  message = nil
  try
    check()
  catch error
    message = error.to_s
  end
  message
end

describe("Crypto.ledger_hash") do
  test("equals sha256 over prev:seq:canonical_json") do
    assert_eq(Crypto.ledger_hash("genesis", 1, {"b": 2, "a": 1}), sha256("genesis:1:{\"a\":1,\"b\":2}"))
  end

  test("is deterministic and changes with seq and prev") do
    data = {"amount": 10}
    first = Crypto.ledger_hash("prev", 1, data)
    assert_eq(Crypto.ledger_hash("prev", 1, data), first)
    assert_ne(Crypto.ledger_hash("prev", 2, data), first)
    assert_ne(Crypto.ledger_hash("genesis", 1, data), first)
  end
end

describe("Crypto.merkle_root") do
  test("an empty list hashes the empty string") do
    assert_eq(Crypto.merkle_root([]), sha256(""))
  end

  test("a single leaf is its own root") do
    leaf = sha256("only leaf")
    assert_eq(Crypto.merkle_root([leaf]), leaf)
  end

  test("two leaves combine as sha256(left_hex + right_hex)") do
    leaf_a = sha256("a")
    leaf_b = sha256("b")
    assert_eq(Crypto.merkle_root([leaf_a, leaf_b]), sha256(leaf_a + leaf_b))
  end

  test("an odd node pairs with itself (Bitcoin convention)") do
    leaf_a = sha256("a")
    leaf_b = sha256("b")
    leaf_c = sha256("c")
    expected = sha256(sha256(leaf_a + leaf_b) + sha256(leaf_c + leaf_c))
    assert_eq(Crypto.merkle_root([leaf_a, leaf_b, leaf_c]), expected)
  end

  test("changes when any leaf changes") do
    leaves = [sha256("tx1"), sha256("tx2"), sha256("tx3")]
    tampered = [sha256("tx1"), sha256("EVIL"), sha256("tx3")]
    assert_ne(Crypto.merkle_root(leaves), Crypto.merkle_root(tampered))
  end
end

describe("Hex") do
  test("encode turns a string's UTF-8 bytes into lowercase hex") do
    assert_eq(Hex.encode("hello"), "68656c6c6f")
    assert_eq(Hex.encode("é"), "c3a9")
    assert_eq(Hex.encode(""), "")
  end

  test("encode accepts a byte array") do
    assert_eq(Hex.encode([0, 255, 16]), "00ff10")
  end

  test("decode returns the byte array, in either case") do
    assert_eq(Hex.decode("68656c6c6f"), [104, 101, 108, 108, 111])
    assert_eq(Hex.decode("C3A9"), [195, 169])
    assert_eq(Hex.encode(Hex.decode("00ff10")), "00ff10")
  end

  test("decode refuses odd-length and non-hex input") do
    assert_raises("Hex.decode(): odd-length hex string") do
      Hex.decode("abc")
    end
    assert_raises("Hex.decode(): invalid hex byte 'zz'") do
      Hex.decode("zz")
    end
  end
end

describe("RsaKey.public_from_pem and X509.spki_pin") do
  test("public_from_pem extracts the same modulus and exponent as private_from_pem") do
    private_key = RsaKey.private_from_pem(RSA_KEY_PEM)
    public_key = RsaKey.public_from_pem(RSA_PUBLIC_PEM)
    assert_eq(public_key["algorithm"], "RSA")
    assert_eq(public_key["bits"], 2048)
    assert_eq(public_key["e"], "010001")
    assert_eq(public_key["n"], private_key["n"])
    assert_eq(public_key["e"], private_key["e"])
  end

  test("spki_pin is the base64 sha256 of the certificate's SubjectPublicKeyInfo") do
    # Cross-checked with: openssl x509 -pubkey | openssl pkey -pubin -outform der | openssl dgst -sha256 -binary
    assert_eq(X509.spki_pin(CERT_PEM_A), "sha256/Sbn6y+nHDTM7p2ErczMi3osqOZN4QbWZvsSaLC5C5tk=")
  end

  test("spki_pin reads raw DER base64 too and tracks the key material") do
    assert_eq(X509.spki_pin(CERT_DER_B64_B), "sha256/akSS/A6LJwtylkUDBEcdo8Ql6Xs5HFh7oPPv0JJGPEA=")
  end
end

describe("UUID generation") do
  test("uuid_v4 returns a distinct hyphenated v4 UUID") do
    first = uuid_v4()
    assert_match(first, "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
    assert_ne(uuid_v4(), first)
  end

  test("uuid_v7 returns a v7 UUID") do
    assert_match(uuid_v7(), "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
  end

  test("UUID.v4 and UUID.v7 behave like the globals") do
    v4_value = UUID.v4
    assert_match(v4_value, "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
    assert_match(UUID.v7, "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
    assert_ne(UUID.v4, v4_value)
  end
end

describe("ULID generation") do
  test("ulid returns a distinct 26-char Crockford Base32 identifier") do
    first = ulid()
    assert_match(first, "^[0-9ABCDEFGHJKMNPQRSTVWXYZ]{26}$")
    assert_ne(ulid(), first)
  end

  test("ULID.generate and ULID.new produce the same format") do
    assert_match(ULID.generate, "^[0-9ABCDEFGHJKMNPQRSTVWXYZ]{26}$")
    assert_match(ULID.new, "^[0-9ABCDEFGHJKMNPQRSTVWXYZ]{26}$")
  end

  test("the 10-char timestamp prefix never goes backwards") do
    earlier = ULID.generate
    later = ULID.new
    assert(earlier.substring(0, 10) <= later.substring(0, 10))
  end
end

describe("NanoID generation") do
  test("nanoid defaults to 21 URL-safe chars") do
    assert_match(nanoid(), "^[A-Za-z0-9_-]{21}$")
    assert_ne(nanoid(), nanoid())
  end

  test("nanoid honors a size and a custom alphabet") do
    assert_match(nanoid(10), "^[A-Za-z0-9_-]{10}$")
    assert_match(nanoid(8, "abc"), "^[abc]{8}$")
    assert_eq(nanoid(4, "x"), "xxxx")
  end

  test("NanoID.generate and NanoID.new take the same arguments") do
    assert_match(NanoID.generate(), "^[A-Za-z0-9_-]{21}$")
    assert_match(NanoID.generate(5), "^[A-Za-z0-9_-]{5}$")
    assert_eq(NanoID.new(3, "z"), "zzz")
    assert_match(NanoID.new(), "^[A-Za-z0-9_-]{21}$")
  end

  test("NanoID.generate without parentheses generates an id") do
    pending("bug: a variadic native static (NanoID.generate, Crypto.random_token) is not called without ()")
    assert_eq(NanoID.generate.length, 21)
  end

  test("a size outside 1..1024 and an empty alphabet are refused") do
    assert_raises("nanoid size must be between 1 and 1024, got 0") do
      NanoID.generate(0)
    end
    assert_raises("nanoid size must be between 1 and 1024, got 1025") do
      nanoid(1025)
    end
    assert_raises("nanoid alphabet cannot be empty") do
      NanoID.generate(5, "")
    end
    assert_raises("nanoid() takes 0-2 args, got 3") do
      nanoid(1, "a", 3)
    end
  end

  test("a non-ASCII alphabet is honored") do
    pending("bug: nanoid(3, \"é\") never returns (any alphabet with a multi-byte char hangs)")
    assert_eq(nanoid(3, "é"), "ééé")
  end
end

work_dir = nil

describe("filesystem write helpers") do
  before_each() do
    work_dir = fresh_tmp_dir()
    mkdir_p(work_dir)
  end

  after_each() do
    ["bytes.bin", "decoded.bin"].each do |name|
      File.delete("#{work_dir}/#{name}") if file_exists("#{work_dir}/#{name}")
    end
  end

  test("file_write_bytes writes raw bytes and file_exists sees them") do
    path = "#{work_dir}/bytes.bin"
    assert_not(file_exists(path))
    assert_eq(file_write_bytes(path, [0, 1, 2, 255]), true)
    assert(file_exists(path))
    assert_eq(slurp(path, "binary"), [0, 1, 2, 255])
  end

  test("file_write_base64 decodes and writes binary content") do
    path = "#{work_dir}/decoded.bin"
    assert_eq(file_write_base64(path, "AAECAw=="), true)
    assert_eq(slurp(path, "binary"), [0, 1, 2, 3])
  end
end

describe("file globbing and modification time") do
  # Fixture: alpha.txt, data.bin and logs/deep.txt
  before_each() do
    work_dir = fresh_tmp_dir()
    mkdir_p("#{work_dir}/logs")
    file_write_bytes("#{work_dir}/alpha.txt", [104, 101, 108, 108, 111])
    file_write_base64("#{work_dir}/data.bin", "AAECAw==")
    barf("#{work_dir}/logs/deep.txt", "deep content")
  end

  # The directories stay behind: there is no builtin to remove one.
  after_each() do
    ["alpha.txt", "data.bin", "logs/deep.txt"].each do |name|
      File.delete("#{work_dir}/#{name}") if file_exists("#{work_dir}/#{name}")
    end
  end

  test("File.glob lists the matching paths of one directory") do
    assert_eq(File.glob("#{work_dir}/*.txt"), ["#{work_dir}/alpha.txt"])
    assert_eq(File.glob("#{work_dir}/*").sort(), ["#{work_dir}/alpha.txt", "#{work_dir}/data.bin", "#{work_dir}/logs"])
  end

  test("File.glob returns an empty list for a pattern that matches nothing") do
    assert_eq(File.glob("#{work_dir}/*.md"), [])
    assert_eq(File.glob("#{work_dir}/*.bin"), ["#{work_dir}/data.bin"])
  end

  test("File.glob raises on a directory that does not exist") do
    assert_raises("File.glob() failed to read directory") do
      File.glob("#{work_dir}/missing/*.txt")
    end
  end

  test("File.glob_recursive descends into subdirectories") do
    assert_eq(File.glob_recursive("#{work_dir}/*.txt").sort(), ["#{work_dir}/alpha.txt", "#{work_dir}/logs/deep.txt"])
  end

  test("File.modified returns the mtime in epoch seconds") do
    modified_at = File.modified("#{work_dir}/alpha.txt")
    assert_gt(modified_at, 1600000000)
    assert_lt(modified_at, 4102444800)
  end

  test("Trusted.glob mirrors File.glob") do
    assert_eq(Trusted.glob("#{work_dir}/*.txt"), File.glob("#{work_dir}/*.txt"))
  end

  test("Trusted.glob_recursive finds nested files") do
    expected = ["#{work_dir}/alpha.txt", "#{work_dir}/logs/deep.txt"]
    assert_eq(Trusted.glob_recursive("#{work_dir}/*.txt").sort(), expected)
  end

  test("Trusted.modified agrees with File.modified") do
    assert_eq(Trusted.modified("#{work_dir}/logs/deep.txt"), File.modified("#{work_dir}/logs/deep.txt"))
  end
end

describe("Logger.warn and the capture buffer") do
  before_each() do
    Logger.set_capture(true)
    Logger.clear_entries()
  end

  after_each() do
    Logger.configure({"level": "info"})
    Logger.set_capture(false)
    Logger.clear_entries()
  end

  test("Logger.warn records a WARN entry with its context") do
    Logger.warn("disk almost full", {"used_pct": 91})
    entries = Logger.entries()
    assert_eq(entries.length, 1)
    assert_contains(entries[0], "[WARN]")
    assert_contains(entries[0], "disk almost full")
    assert_contains(entries[0], "used_pct=91")
  end

  test("clear_entries empties the buffer") do
    Logger.warn("one")
    Logger.clear_entries()
    assert_eq(Logger.entries(), [])
  end

  test("a warning below the configured level is dropped") do
    Logger.configure({"level": "error"})
    Logger.warn("should not be captured")
    assert_eq(Logger.entries(), [])
  end
end

describe("Factory DSL") do
  after_each() do
    Factory.clear()
  end

  test("create_list builds N hashes with the sequence interpolated") do
    Factory.define("cif_user", {"email": r"user#{n}@example.com", "role": "member"})
    users = Factory.create_list("cif_user", 3)
    assert_eq(users.map { |user| user["email"] }, ["user0@example.com", "user1@example.com", "user2@example.com"])
    assert_eq(users[2]["role"], "member")
  end

  test("create_list of zero yields an empty array") do
    Factory.define("cif_empty", {"name": "nobody"})
    assert_eq(Factory.create_list("cif_empty", 0), [])
  end

  test("sequence advances by one per name, independently") do
    first = Factory.sequence("cif_seq")
    other = Factory.sequence("cif_other_seq")
    assert_eq(Factory.sequence("cif_seq"), first + 1)
    assert_eq(Factory.sequence("cif_other_seq"), other + 1)
  end
end

describe("expect(...).to_match") do
  test("passes on a substring match") do
    expect("hello world").to_match("world")
    expect(uuid_v4()).to_match("-")
  end

  test("fails on a mismatch") do
    message = assertion_failure(fn() { expect("hello").to_match("zzz-not-there") })
    assert_contains(message, "Expected \"hello\" to match \"zzz-not-there\"")
  end
end

describe("standalone string globals and puts") do
  test("contains checks substrings") do
    assert(contains("hello world", "lo wo"))
    assert_not(contains("hello world", "goodbye"))
  end

  test("starts_with and ends_with check affixes") do
    assert(starts_with("soli.txt", "soli"))
    assert_not(starts_with("soli.txt", ".txt"))
    assert(ends_with("soli.txt", ".txt"))
    assert_not(ends_with("soli.txt", "soli"))
  end

  test("replace substitutes every non-overlapping occurrence") do
    assert_eq(replace("a-b-c", "-", "+"), "a+b+c")
    assert_eq(replace("aaa", "aa", "b"), "ba")
    assert_eq(replace("unchanged", "zzz", "y"), "unchanged")
  end

  test("puts writes a line and returns nil") do
    assert_null(puts("crypto_ids_files_spec puts probe"))
  end
end

describe("comparison and match assertions") do
  test("assert_gt, assert_lt, assert_ne and assert_match pass on valid input") do
    assert_gt(5, 4)
    assert_lt(4, 5)
    assert_ne("one", "two")
    assert_match("hello.sol", "\\.sol$")
  end

  test("assert_gt fails when the first value is not greater") do
    assert_contains(assertion_failure(fn() { assert_gt(3, 3) }), "expected 3 to be greater than 3")
  end

  test("assert_lt fails when the first value is not smaller") do
    assert_contains(assertion_failure(fn() { assert_lt(5, 4) }), "expected 5 to be less than 4")
  end

  test("assert_ne fails on equal values") do
    assert_contains(assertion_failure(fn() { assert_ne(1, 1) }), "expected a value other than 1")
  end

  test("assert_match fails when the pattern does not match") do
    message = assertion_failure(fn() { assert_match("hello", "^\\d+$") })
    assert_contains(message, "expected \"hello\" to match /^\\d+$/")
  end
end
