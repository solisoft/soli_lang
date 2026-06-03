# ============================================================================
# RSA primitives (modexp + PKCS#1 v1.5) and exclusive XML C14N test suite.
#
# Together these three primitives are the building blocks of XML-DSig /
# WS-Security signing in Soli.
# ============================================================================

# 512-bit RSA test vector (test-only key, generated deterministically).
const RSA_N = "980a9aca7c4d829702a0b0da629e0277baa5e3c5f6f91a75670f806111efafbf0f98453957966b7894d4bff4df3a6e1b105e28efe7583c0ed1ffad48e0ae81f9"
const RSA_E = "010001"
const RSA_D = "69591fbc21b90b3d5b62c067f1610ed0ab117aeb969f30081d2b0e873408623ae7efe61003b265257094bc5617f40c5ad67bdab63afc9da1ce50064949e0e069"
const RSA_K = 64

describe("Crypto.modexp", fn() {
    test("computes textbook modular exponentiation", fn() {
        # 4^13 mod 497 = 445 -> 0x01bd (modulus 0x01f1 is 2 octets wide)
        assert_eq(Crypto.modexp("04", "0d", "01f1"), "01bd");
    });

    test("accepts a 0x prefix and byte arrays", fn() {
        assert_eq(Crypto.modexp("0x04", [13], [1, 241]), "01bd");
    });

    test("left-pads the result to the modulus octet width", fn() {
        # 2^3 mod 0x01f1 = 8 -> "0008", two octets wide.
        assert_eq(Crypto.modexp("02", "03", "01f1"), "0008");
    });
});

describe("Crypto.pkcs1_pad / pkcs1_unpad", fn() {
    test("type 1 padding round-trips", fn() {
        let data = "deadbeef";
        let em = Crypto.pkcs1_pad(data, RSA_K);          # block type 1 default
        assert_eq(len(em), RSA_K * 2);                   # hex chars = 2 * octets
        assert_eq(em.substring(0, 4), "0001");           # 0x00 0x01 prefix
        assert_eq(Crypto.pkcs1_unpad(em), data);
    });

    test("type 2 padding round-trips and is randomized", fn() {
        let data = "deadbeef";
        let a = Crypto.pkcs1_pad(data, RSA_K, 2);
        let b = Crypto.pkcs1_pad(data, RSA_K, 2);
        assert_eq(a.substring(0, 4), "0002");            # 0x00 0x02 prefix
        assert(a != b);                                   # random padding differs
        assert_eq(Crypto.pkcs1_unpad(a), data);
    });
});

describe("RSA sign/verify round-trip (modexp + PKCS#1)", fn() {
    test("a padded hash signed with d verifies with e", fn() {
        let digest = Crypto.sha256("<doc></doc>");        # 32-byte hex hash

        # Sign: pad the digest, then raise to the private exponent d mod n.
        let em = Crypto.pkcs1_pad(digest, RSA_K);
        let signature = Crypto.modexp(em, RSA_D, RSA_N);

        # Verify: raise the signature to the public exponent e, strip padding.
        let recovered_em = Crypto.modexp(signature, RSA_E, RSA_N);
        let recovered = Crypto.pkcs1_unpad(recovered_em);

        assert_eq(recovered, digest);
    });
});

describe("Xml.c14n_exclusive", fn() {
    test("expands empty elements and drops the XML declaration", fn() {
        assert_eq(
            Xml.c14n_exclusive("<?xml version=\"1.0\"?><doc/>"),
            "<doc></doc>"
        );
    });

    test("sorts attributes and uses double quotes", fn() {
        assert_eq(
            Xml.c14n_exclusive("<doc b='2' a='1'></doc>"),
            "<doc a=\"1\" b=\"2\"></doc>"
        );
    });

    test("drops namespaces not visibly utilized (the exclusive rule)", fn() {
        let xml = "<n0:root xmlns:n0=\"http://a\" xmlns:n2=\"http://c\"><n1:e xmlns:n1=\"http://b\">x</n1:e></n0:root>";
        let canonical = Xml.c14n_exclusive(xml);
        assert_not(canonical.includes?("http://c"));      # unused n2 removed
        assert(canonical.includes?("xmlns:n1=\"http://b\""));
    });

    test("honors an InclusiveNamespaces prefix list", fn() {
        let xml = "<n0:root xmlns:n0=\"http://a\" xmlns:n2=\"http://c\"><n0:child>t</n0:child></n0:root>";
        let canonical = Xml.c14n_exclusive(xml, "n2");
        assert(canonical.includes?("xmlns:n2=\"http://c\""));
    });

    test("is stable: canonicalizing twice is idempotent", fn() {
        let xml = "<a:x xmlns:a='http://a'  b='2'  a='1' >  hi  </a:x>";
        let once = Xml.c14n_exclusive(xml);
        let twice = Xml.c14n_exclusive(once);
        assert_eq(once, twice);
    });
});
