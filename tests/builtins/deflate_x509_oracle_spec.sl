# Deflate and X509 cross-validated against Python (zlib raw DEFLATE,
# `cryptography`) and openssl.

# Raw-DEFLATE(xml) produced by Python zlib (wbits=-15), base64'd — the exact
# SAML HTTP-Redirect SAMLRequest encoding
const SAML_PARAM = "sylOzM0psHIsLcnIC0otLE0tLlGoyM3JK7YCS9gqlRblWeUnFmcWW+Ul5qYWW5UkWwU7+vpYGekZWBUU5" +
  "ZfkJ+fnKCl4utgqxRcZKunbAQA="
const EXPECTED_XML = "<samlp:AuthnRequest xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\" ID=\"_r1\"/>"

# A PEM certificate (multi-line) and the n/e of its key
const CERT_PEM = "-----BEGIN CERTIFICATE-----\n" +
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
const PEM_N = "b40265db78674611afa49e5043885a2df58ade68ddf1295ff1f9e15160ff433a28dc8d5bb2f72bf0702392" +
  "96882a77b968b5358e9fe68a0bdc7600f1df7240724db723ba73f58ab8eec280d7f28ff931719b3380034f0cdf8c593b804f" +
  "0f55560c1359c9fc2d5f9140f1e422b8123450890b92cba52dbf703e3a9ddbcbc3005565746be55cc4115591169f463bc6a0" +
  "b680608d00cdb2ecdde80cd3ef008b88b2d14d184c61f9b3ff27b3c365e4c8851daad45b06a1e67018c3b553c1332508fe0f" +
  "665a85f17668a5ed1cea39d75110b265c4c7f0e1a682ab82f0a5bfdfe3669631c52994a1cb180a21108118bfd1869d7d3273" +
  "2c05061a030d44681fce17e2ab"
const PEM_E = "010001"

describe("Deflate interop with Python zlib (SAML Redirect binding)") do
  test("inflate decodes a Python raw-DEFLATE SAMLRequest") do
    assert_eq(Deflate.inflate(Base64.decode(SAML_PARAM)), EXPECTED_XML)
  end

  test("deflate then inflate round-trips through Base64") do
    param = Base64.encode(Deflate.deflate(EXPECTED_XML))
    assert_eq(Deflate.inflate(Base64.decode(param)), EXPECTED_XML)
  end

  test("deflate returns raw bytes and compresses repetition") do
    assert_eq(Deflate.deflate(""), [3, 0])
    assert_eq(Deflate.inflate(Deflate.deflate("")), "")
    assert_eq(Deflate.deflate("a" * 80).length, 6)
  end

  test("inflate refuses a corrupt stream") do
    assert_raises("inflate error: corrupt deflate stream") do
      Deflate.inflate("not deflate")
    end
  end
end

describe("X509 on PEM input") do
  test("public_key extracts the modulus and exponent") do
    key = X509.public_key(CERT_PEM)
    assert_eq(key["n"], PEM_N)
    assert_eq(key["e"], PEM_E)
  end

  test("public_key refuses input that is neither PEM, base64 nor hex") do
    assert_raises("certificate is neither valid base64 nor hex") do
      X509.public_key("garbage")
    end
  end

  test("fingerprint is the sha256 of the DER by default, as openssl prints it") do
    assert_eq(X509.fingerprint(CERT_PEM), "d5268d63e36943d4a566f087fca79eea6989ee5384a584aab5b451272d2b21fc")
  end

  test("fingerprint takes sha1 and refuses other algorithms") do
    assert_eq(X509.fingerprint(CERT_PEM, "SHA1"), "a65a7864c016d71103b75dd98b666904c2aae9f8")
    assert_raises("unsupported algorithm 'md5' (use sha256 or sha1)") do
      X509.fingerprint(CERT_PEM, "md5")
    end
  end
end
