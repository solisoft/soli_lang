# RSA primitives (Crypto.modexp, PKCS#1 v1.5 padding) and exclusive XML C14N:
# the building blocks of XML-DSig / WS-Security signing in Soli.

# 512-bit RSA test key (test-only, generated deterministically)
const RSA_N = "980a9aca7c4d829702a0b0da629e0277baa5e3c5f6f91a75670f806111efafbf" +
  "0f98453957966b7894d4bff4df3a6e1b105e28efe7583c0ed1ffad48e0ae81f9"
const RSA_E = "010001"
const RSA_D = "69591fbc21b90b3d5b62c067f1610ed0ab117aeb969f30081d2b0e873408623a" +
  "e7efe61003b265257094bc5617f40c5ad67bdab63afc9da1ce50064949e0e069"
const RSA_K = 64

describe("Crypto.modexp") do
  test("computes textbook modular exponentiation") do
    # 4^13 mod 497 = 445 -> 0x01bd (modulus 0x01f1 is 2 octets wide)
    assert_eq(Crypto.modexp("04", "0d", "01f1"), "01bd")
  end

  test("accepts a 0x prefix and byte arrays") do
    assert_eq(Crypto.modexp("0x04", [13], [1, 241]), "01bd")
  end

  test("left-pads the result to the modulus octet width") do
    # 2^3 mod 0x01f1 = 8 -> "0008"
    assert_eq(Crypto.modexp("02", "03", "01f1"), "0008")
  end

  test("a zero exponent gives 1") do
    assert_eq(Crypto.modexp("05", "00", "07"), "01")
  end

  test("refuses a zero modulus") do
    assert_raises("Crypto.modexp(): modulus must be non-zero") do
      Crypto.modexp("05", "01", "00")
    end
  end
end

describe("Crypto.pkcs1_pad / pkcs1_unpad") do
  test("type 1 padding is 00 01, FF octets, 00, then the data") do
    encoded = Crypto.pkcs1_pad("deadbeef", RSA_K)
    assert_eq(encoded, "0001" + "ff" * 57 + "00" + "deadbeef")
    assert_eq(Crypto.pkcs1_unpad(encoded), "deadbeef")
  end

  test("type 2 padding round-trips and is randomized") do
    first = Crypto.pkcs1_pad("deadbeef", RSA_K, 2)
    second = Crypto.pkcs1_pad("deadbeef", RSA_K, 2)
    assert_eq(first.length, RSA_K * 2)
    assert_eq(first.substring(0, 4), "0002")
    assert_ne(first, second)
    assert_eq(Crypto.pkcs1_unpad(first), "deadbeef")
  end

  test("refuses data longer than the key allows") do
    assert_raises("data length 60 too long for key size 64 (max 53 octets)") do
      Crypto.pkcs1_pad("00" * 60, RSA_K)
    end
  end

  test("refuses a block type other than 1 or 2") do
    assert_raises("block_type must be 1 or 2, got 3") do
      Crypto.pkcs1_pad("dead", RSA_K, 3)
    end
  end

  test("unpad refuses a short, mistyped or malformed block") do
    assert_raises("encoded message too short (need >= 11 octets)") do
      Crypto.pkcs1_unpad("0003ffff00aa")
    end
    assert_raises("unsupported block type 0x03 (expected 0x01 or 0x02)") do
      Crypto.pkcs1_unpad("0003" + "ff" * 10 + "00aa")
    end
    assert_raises("block type 1 padding octet is not 0xFF") do
      Crypto.pkcs1_unpad("0001" + "ff" * 10 + "aa")
    end
  end
end

describe("RSA sign/verify round-trip (modexp + PKCS#1)") do
  test("a padded hash signed with d verifies with e") do
    digest = Crypto.sha256("<doc></doc>")
    signature = Crypto.modexp(Crypto.pkcs1_pad(digest, RSA_K), RSA_D, RSA_N)
    assert_eq(signature.length, RSA_K * 2)
    assert_eq(Crypto.pkcs1_unpad(Crypto.modexp(signature, RSA_E, RSA_N)), digest)
  end
end

describe("Xml.c14n_exclusive") do
  test("expands empty elements and drops the XML declaration") do
    assert_eq(Xml.c14n_exclusive("<?xml version=\"1.0\"?><doc/>"), "<doc></doc>")
  end

  test("sorts attributes and uses double quotes") do
    assert_eq(Xml.c14n_exclusive("<doc b='2' a='1'></doc>"), "<doc a=\"1\" b=\"2\"></doc>")
  end

  test("drops namespaces not visibly utilized (the exclusive rule)") do
    xml = "<n0:root xmlns:n0=\"http://a\" xmlns:n2=\"http://c\"><n1:e xmlns:n1=\"http://b\">x</n1:e></n0:root>"
    assert_eq(Xml.c14n_exclusive(xml), "<n0:root xmlns:n0=\"http://a\"><n1:e xmlns:n1=\"http://b\">x</n1:e></n0:root>")
  end

  test("honors an InclusiveNamespaces prefix list") do
    xml = "<n0:root xmlns:n0=\"http://a\" xmlns:n2=\"http://c\"><n0:child>t</n0:child></n0:root>"
    assert_eq(Xml.c14n_exclusive(xml, "n2"), xml)
  end

  test("normalizes whitespace inside tags but keeps text, and is idempotent") do
    canonical = Xml.c14n_exclusive("<a:x xmlns:a='http://a'  b='2'  a='1' >  hi  </a:x>")
    assert_eq(canonical, "<a:x xmlns:a=\"http://a\" a=\"1\" b=\"2\">  hi  </a:x>")
    assert_eq(Xml.c14n_exclusive(canonical), canonical)
  end

  test("keeps character references escaped and drops comments") do
    assert_eq(Xml.c14n_exclusive("<doc>a &amp; b &lt; c</doc>"), "<doc>a &amp; b &lt; c</doc>")
    assert_eq(Xml.c14n_exclusive("<doc><!-- c --><x/></doc>"), "<doc><x></x></doc>")
  end

  test("refuses input with no document element") do
    assert_raises("Xml.c14n_exclusive(): no document element found") do
      Xml.c14n_exclusive("<doc>")
    end
  end
end
