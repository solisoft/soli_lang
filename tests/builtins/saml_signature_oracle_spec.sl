# ============================================================================
# END-TO-END SAML signature verification, cross-validated against `signxml`
# (Python XML-DSig). The document below was signed by signxml with an enveloped
# signature, exclusive C14N, and RSA-SHA256 — the exact profile SAML uses.
#
# This single test exercises ALL FOUR new capabilities together:
#   X509.public_key  +  Xml.get_elements_by_tag  +  by-id/enveloped c14n  +  modexp/pkcs1
# If Soli's verification agrees with signxml's, the primitives are correct.
# ============================================================================

const N = "bb07599b3d36eba6e9eca29bddd1cb37acca64b2ba5fe4a59ec3d17c4e62f3e6e09c219a61abf034434af10bc356fcbfeac179bb1bcd12a4312370cfd140007799abbb5320a48c2c72142430e122d7d42bb74b1ad26fe24e9271de9aeb9fd73b6ec3cfa920fca202003ddda947a742873991bfb87ae10aba75bb31998d61c0229104fc738e8c7791ec38158c64512df299b98fe0d6e0a4c5c359e109f2e0acdf87f0ca6ebaab37e66566bed5d38333a54260fc2db225a10dc8ecd269af1fd27edbfb9b035b917445a412302a17f018b2179174cdb05a266f405fe278cd580721b6f15cdce24b7680a825a05c1db1c4d9c8e8555aecdebf99e02861060dca5c3f"
const E = "010001"
const CERT = "MIICwDCCAaigAwIBAgIUVcpK7h6+YO8N5bgARxotBOG/xe4wDQYJKoZIhvcNAQELBQAwGjEYMBYGA1UEAwwPaWRwLmV4YW1wbGUuY29tMB4XDTIwMDEwMTAwMDAwMFoXDTM1MDEwMTAwMDAwMFowGjEYMBYGA1UEAwwPaWRwLmV4YW1wbGUuY29tMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAuwdZmz0266bp7KKb3dHLN6zKZLK6X+SlnsPRfE5i8+bgnCGaYavwNENK8QvDVvy/6sF5uxvNEqQxI3DP0UAAd5mru1MgpIwschQkMOEi19Qrt0sa0m/iTpJx3prrn9c7bsPPqSD8ogIAPd2pR6dChzmRv7h64Qq6dbsxmY1hwCKRBPxzjox3kew4FYxkUS3ymbmP4NbgpMXDWeEJ8uCs34fwym66qzfmZWa+1dODM6VCYPwtsiWhDcjs0mmvH9J+2/ubA1uRdEWkEjAqF/AYsheRdM2wWiZvQF/ieM1YByG28Vzc4kt2gKgloFwdscTZyOhVWuzev5ngKGEGDcpcPwIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQCeCSJ3V9LnHv2wg9j1LrMMhr3073o0SvcvXB02sBTWbgjD8Q7gPySpOTOPSr8voRF+O8mvPfAHnjyKnA1fwSsdyAQLvk4H132PcBjlLLwzpO7UxYQlbIr6uIPpylXzt0tNT9gkzZLBRwjgdOsebYh64Ef1cy4CIaLKdikBqVYB1NFHGeLQ1Viy13jVAQRFkD8bEc7RapLQitZ0rdhBp7LwUVYwRXKKGaiSf4hUME99PII3IB0xAPGZ6Kk3sQwakqfGaQm526Ex86+p2A6lsfQ65vJVABpdbjBt8YhXfUAOrb5d9tasZrygybyW810yk5A6WaxZHpO/0DKudtbD221a"
const SIGNED_XML = "<Document ID=\"_obj1\"><Data>Hello SAML</Data><ds:Signature xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\"><ds:SignedInfo><ds:CanonicalizationMethod Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"/><ds:SignatureMethod Algorithm=\"http://www.w3.org/2001/04/xmldsig-more#rsa-sha256\"/><ds:Reference URI=\"#_obj1\"><ds:Transforms><ds:Transform Algorithm=\"http://www.w3.org/2000/09/xmldsig#enveloped-signature\"/><ds:Transform Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"/></ds:Transforms><ds:DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha256\"/><ds:DigestValue>F0TZGbzxdSvvMZ8bE/oI8AzKnqrQMzZj2xl1/d5SgE4=</ds:DigestValue></ds:Reference></ds:SignedInfo><ds:SignatureValue>inIboZlWQW3IXaJ5691+ewoa5UJy5g+unndvgEv3UkfUku997wiJ6IySjjnSiRn5b3ZqDPoyjhMWx2azeEg2rx01VJ55GiHQFhwzq34E6isHhOq+siH4W1LgWe3Gu28JQ7o/jD0e8WU7fVKyRxvf3PqbkjyHACJ3yEsyjX3SZki6Cjb/QbXpVIvR/OGV+0KIZEIItYbhgKX5CnAigJvl8IzRqkGMERNYokpzwZacgIFtW+48u/Sugem+BuztLpeSIB9E/WLYG2Nm9Gdb4xmG6GtmBJVC6G0+TxnDHgSkkpa3aQqePEKZ1fiVKgVJIEj3ygEVtivRTT/RFYsR203TrQ==</ds:SignatureValue><ds:KeyInfo><ds:X509Data><ds:X509Certificate>MIICwDCCAaigAwIBAgIUVcpK7h6+YO8N5bgARxotBOG/xe4wDQYJKoZIhvcNAQELBQAwGjEYMBYGA1UEAwwPaWRwLmV4YW1wbGUuY29tMB4XDTIwMDEwMTAwMDAwMFoXDTM1MDEwMTAwMDAwMFowGjEYMBYGA1UEAwwPaWRwLmV4YW1wbGUuY29tMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAuwdZmz0266bp7KKb3dHLN6zKZLK6X+SlnsPRfE5i8+bgnCGaYavwNENK8QvDVvy/6sF5uxvNEqQxI3DP0UAAd5mru1MgpIwschQkMOEi19Qrt0sa0m/iTpJx3prrn9c7bsPPqSD8ogIAPd2pR6dChzmRv7h64Qq6dbsxmY1hwCKRBPxzjox3kew4FYxkUS3ymbmP4NbgpMXDWeEJ8uCs34fwym66qzfmZWa+1dODM6VCYPwtsiWhDcjs0mmvH9J+2/ubA1uRdEWkEjAqF/AYsheRdM2wWiZvQF/ieM1YByG28Vzc4kt2gKgloFwdscTZyOhVWuzev5ngKGEGDcpcPwIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQCeCSJ3V9LnHv2wg9j1LrMMhr3073o0SvcvXB02sBTWbgjD8Q7gPySpOTOPSr8voRF+O8mvPfAHnjyKnA1fwSsdyAQLvk4H132PcBjlLLwzpO7UxYQlbIr6uIPpylXzt0tNT9gkzZLBRwjgdOsebYh64Ef1cy4CIaLKdikBqVYB1NFHGeLQ1Viy13jVAQRFkD8bEc7RapLQitZ0rdhBp7LwUVYwRXKKGaiSf4hUME99PII3IB0xAPGZ6Kk3sQwakqfGaQm526Ex86+p2A6lsfQ65vJVABpdbjBt8YhXfUAOrb5d9tasZrygybyW810yk5A6WaxZHpO/0DKudtbD221a</ds:X509Certificate></ds:X509Data></ds:KeyInfo></ds:Signature></Document>"
const SIG_HEX = "8a721ba19956416dc85da279ebdd7e7b0a1ae54272e60fae9e776f804bf75247d492ef7def0889e88c928e39d28919f96f766a0cfa328e1316c766b3784836af1d35549e791a21d0161c33ab7e04ea2b0784eabeb221f85b52e059edc6bb6f0943ba3f8c3d1ef1653b7d52b2471bdfdcfa9b923c87002277c84b328d7dd26648ba0a36ff41b5e9548bd1fce195fb4288644208b586e180a5f90a7022809be5f08cd1aa418c111358a24a73c1969c80816d5bee3cbbf4ae81e9be06eced2e9792201f44fd62d81b6366f4675be31986e86b66049542e86d3e4f19c31e04a49296b7690a9e3c4299d5f8952a05492048f7ca0115b62bd14d3fd1158b11db4dd3ad"
const DIGEST_HEX = "1744d919bcf1752bef319f1b13fa08f00cca9eaad0333663db1975fdde52804e"
const SIGNEDINFO_C14N_SHA256 = "149503556fd0c012dbf309abe409fd9b4069361e4a4ae4494b78f9336d7a7dc6"
const SHA256_DIGESTINFO = "3031300d060960864801650304020105000420"

describe("SAML signature verification (signxml oracle)", fn() {
    test("X509.public_key extracts the IdP modulus and exponent", fn() {
        let key = X509.public_key(CERT);
        assert_eq(key["algorithm"], "RSA");
        assert_eq(key["n"], N);
        assert_eq(key["e"], E);
    });

    test("Reference digest: enveloped + by-id exc-c14n matches signxml DigestValue", fn() {
        let canon = Xml.c14n_exclusive(SIGNED_XML, {"id": "_obj1", "enveloped_signature": true});
        assert_eq(Crypto.sha256(canon), DIGEST_HEX);
    });

    test("SignedInfo extraction + exc-c14n hash matches signxml", fn() {
        let signed_info = Xml.get_elements_by_tag(SIGNED_XML, "SignedInfo")[0];
        let canon = Xml.c14n_exclusive(signed_info);
        assert_eq(Crypto.sha256(canon), SIGNEDINFO_C14N_SHA256);
    });

    test("RSA: recovering the signature yields the SignedInfo DigestInfo (full chain)", fn() {
        let key = X509.public_key(CERT);
        let recovered = Crypto.pkcs1_unpad(Crypto.modexp(SIG_HEX, key["e"], key["n"]));
        assert_eq(recovered, SHA256_DIGESTINFO + SIGNEDINFO_C14N_SHA256);
    });
});
