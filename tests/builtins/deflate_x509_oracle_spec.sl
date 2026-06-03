# ============================================================================
# Deflate + X509 cross-validation against Python (zlib raw DEFLATE, cryptography).
# ============================================================================

# Raw-DEFLATE(xml) produced by Python zlib (wbits=-15), base64'd — the exact
# SAML HTTP-Redirect SAMLRequest encoding.
const SAML_PARAM = "sylOzM0psHIsLcnIC0otLE0tLlGoyM3JK7YCS9gqlRblWeUnFmcWW+Ul5qYWW5UkWwU7+vpYGekZWBUU5ZfkJ+fnKCl4utgqxRcZKunbAQA=";
const EXPECTED_XML = "<samlp:AuthnRequest xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\" ID=\"_r1\"/>";

# A PEM certificate (multi-line) and the n/e of its key.
const CERT_PEM = "-----BEGIN CERTIFICATE-----\nMIICqzCCAZOgAwIBAgIBATANBgkqhkiG9w0BAQsFADAZMRcwFQYDVQQDDA5zcC5l\neGFtcGxlLmNvbTAeFw0yMDAxMDEwMDAwMDBaFw0zNTAxMDEwMDAwMDBaMBkxFzAV\nBgNVBAMMDnNwLmV4YW1wbGUuY29tMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIB\nCgKCAQEAtAJl23hnRhGvpJ5QQ4haLfWK3mjd8Slf8fnhUWD/Qzoo3I1bsvcr8HAj\nkpaIKne5aLU1jp/migvcdgDx33JAck23I7pz9Yq47sKA1/KP+TFxmzOAA08M34xZ\nO4BPD1VWDBNZyfwtX5FA8eQiuBI0UIkLksulLb9wPjqd28vDAFVldGvlXMQRVZEW\nn0Y7xqC2gGCNAM2y7N3oDNPvAIuIstFNGExh+bP/J7PDZeTIhR2q1FsGoeZwGMO1\nU8EzJQj+D2ZahfF2aKXtHOo511EQsmXEx/DhpoKrgvClv9/jZpYxxSmUocsYCiEQ\ngRi/0YadfTJzLAUGGgMNRGgfzhfiqwIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQCp\n03SxaAvxD1c+pMg4Q3YTPFUe5eFRvYxaXG8BdDCH9P+uD+TVWstPr5Rx7pWDGsuV\n9QlyNmA3bFea4Ps8n7CiEuiJeDzbtTznOBHbF5/AUj7fhNHu9Su0Ka4Fg5QCuRGZ\nB6Z6fkJDkZ0NVRJXwqXgOByvm7i0VE0mtFaf1kyqApPV2IohF/CxfqsMz/dySWPl\nODWt3qmRBU3Wk5wUtD+71Opmb+qfXZoqFuKoY1MHSf14rXcV/tETLyhXp8oaA/OM\nb9onERfWbd8xx//ct4TFLUo64uvsyXtFhnAcjVec/qOMUzZ/OmzVW8caCThFIhnt\n2StVAKOJevnZUkGGHuyN\n-----END CERTIFICATE-----\n";
const PEM_N = "b40265db78674611afa49e5043885a2df58ade68ddf1295ff1f9e15160ff433a28dc8d5bb2f72bf070239296882a77b968b5358e9fe68a0bdc7600f1df7240724db723ba73f58ab8eec280d7f28ff931719b3380034f0cdf8c593b804f0f55560c1359c9fc2d5f9140f1e422b8123450890b92cba52dbf703e3a9ddbcbc3005565746be55cc4115591169f463bc6a0b680608d00cdb2ecdde80cd3ef008b88b2d14d184c61f9b3ff27b3c365e4c8851daad45b06a1e67018c3b553c1332508fe0f665a85f17668a5ed1cea39d75110b265c4c7f0e1a682ab82f0a5bfdfe3669631c52994a1cb180a21108118bfd1869d7d32732c05061a030d44681fce17e2ab";
const PEM_E = "010001";

describe("Deflate interop with Python zlib (SAML Redirect binding)", fn() {
    test("inflate decodes a Python raw-DEFLATE SAMLRequest", fn() {
        let xml = Deflate.inflate(Base64.decode(SAML_PARAM));
        assert_eq(xml, EXPECTED_XML);
    });

    test("deflate -> inflate round-trips and Base64 pipes cleanly", fn() {
        let param = Base64.encode(Deflate.deflate(EXPECTED_XML));
        let back = Deflate.inflate(Base64.decode(param));
        assert_eq(back, EXPECTED_XML);
    });
});

describe("X509.public_key on PEM input", fn() {
    test("extracts modulus/exponent from a PEM certificate", fn() {
        let key = X509.public_key(CERT_PEM);
        assert_eq(key["n"], PEM_N);
        assert_eq(key["e"], PEM_E);
    });

    test("fingerprint is a 64-char sha256 hex by default", fn() {
        let fp = X509.fingerprint(CERT_PEM);
        assert_eq(len(fp), 64);
    });
});
