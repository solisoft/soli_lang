const KEY_PEM = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCca/ZMr23wsIM9\nLY5nPcV4ViQxdQ30Ca65FZMCZXQYQy+edBMeXeVF34DzVKSZUkyAKZ93U6ShZVkb\nkwvqFJP1jYrzMU+ov2eempYg0N9WDG02NaMw65ZVaKHbR5N4axHHxGLpdZbwVPPl\nsAIm7A1YMk4fF4gtneED4jjXtCVYwzTkB2HWegAYqpSkuzeyp4F/6LFCPOQ5VDqg\nZRNSXCZVQC42Dmt0QMNICqyKjcfTB/IAS1DtRYwqBfADNPS+OGnARd5iU60gBCCL\nBD38G3ZDAI/P3HsonXpfo7w+6sMn2B8V8VPvd9O32p5PB+NrmIcIJpocipIngM0L\nOPLaC8eNAgMBAAECggEAKAb+i4gWz5EzvDuEpcmqVw1gDKHiFLFHm0g4itPwXecP\nb/JPFCW97l/vzRS7XBqxxdgg3PWz+rMHFuXNljR22k7CoFHdixaTywPO6A3bINdk\nOQuHu5SFr0xrosPRqm5nqeGI2CoFmnF6yit8mX4tOgUBdbZdXCL6+jXxCs2oAurm\ne4hfNmrxoG599cpl58p8Yg+JPrsiPWQxeRU7tVNRGUbovT3jYYDsOpcI4qlNZIcd\nfcZHIkI1/5J5B08cWcyc88xIEFUrHFoe01TMXTrt3GosmF+1VSyZGv1P2y7CX7j6\nUqxYyom0nK7eWXI+XbpqIAmLCykMIqofs3lyRyXIJwKBgQDSLtGpcw4XbTVhMjF8\nxjQk2TaI5cky+hr9y19mfHdY/xld9k0aiT8CbNTUhlyFf8ZapwussBhdTBiOKYtQ\nPWyzIyjq99pedep9Ze4RuhFNCMfdogZcj6RT1Bi1tNtedKaCIJCNo/RdmH3t8798\n2lW7KMDC6bQd5pE+bmyvT8fnowKBgQC+hQVRtGBkDNqL2YpC7lZm2O8XKUFbV+Z7\nJZhEZGYaxiCN+1fywB+EL41xGuVmOxlxuOrQ33ugHtlt8YEarX3vUQlxxPXsU+Fm\nECgTXEiAYn9TmNTB/q075Ed5HC5+poyRfgTpNSKeXMvDcMY9Xi4nIIAC1IefJRDU\nOAwc8P1HDwKBgQCBXhPqakjYHn3mj1BqbkyWCaRJarYGTG7km5Lir+V9v7ZLYVhf\n5u4Dfh0ZmoHEIbti/MJwzgqREk9i4StAfi4zrIZ46Ylc7tMfz+dSveX8NlVek2W6\n/yaz+i4jWWhUoRQDsCuJIss7+Ko6Ffdcz75I7nKHBfW5Gbt4Y9s9pKt0ZQKBgEwn\n9iFb3fAAZ1fhxG/Ov8Dq1F/IwPRXZa0yMPSdwWbQbfDzWIuTmsWHEJ32p14/H4Oi\n7FJEEzHFQxq8n+PfF+kS1pigp8EpIn9e0/YxPFX9iXIMNHe7atn2/U7/IeLEhood\n+q6R692rsFPWf5fGTuKbDjCTbgcClQCPyt/CwSunAoGBAM/VB/icg3q/pyRERic2\nXKTomPi/5gruQgSlrHw93k1XObk+jOt1Xba6uDhUiXRxWwRz7lqpZ2l6NflbngNp\nY/0tUksNJ0PRhV3NLdVKsZdE/1fl4kbxNE5VMCUn1IHBFFHSaM4Rk5vfAYBLwAyW\np1uYKl5VM/9SLSt39wJM9nqx\n-----END PRIVATE KEY-----\n"
const CERT_B64 = "MIICqzCCAZOgAwIBAgIBATANBgkqhkiG9w0BAQsFADAZMRcwFQYDVQQDDA5zcC5leGFtcGxlLmNvbTAeFw0yMDAxMDEwMDAwMDBaFw0zNTAxMDEwMDAwMDBaMBkxFzAVBgNVBAMMDnNwLmV4YW1wbGUuY29tMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAnGv2TK9t8LCDPS2OZz3FeFYkMXUN9AmuuRWTAmV0GEMvnnQTHl3lRd+A81SkmVJMgCmfd1OkoWVZG5ML6hST9Y2K8zFPqL9nnpqWINDfVgxtNjWjMOuWVWih20eTeGsRx8Ri6XWW8FTz5bACJuwNWDJOHxeILZ3hA+I417QlWMM05Adh1noAGKqUpLs3sqeBf+ixQjzkOVQ6oGUTUlwmVUAuNg5rdEDDSAqsio3H0wfyAEtQ7UWMKgXwAzT0vjhpwEXeYlOtIAQgiwQ9/Bt2QwCPz9x7KJ16X6O8PurDJ9gfFfFT73fTt9qeTwfja5iHCCaaHIqSJ4DNCzjy2gvHjQIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQBUHvn9HJEMkw4NZ95aSbmNAF+uMoCOCuj9NotJrmbUFdjLpfSvWpx4lSI6ZoUdXa7QyN/Wi7gBJgSl1ex6axSAD3Al5fEZqBuEqsMMncRGf2y463MGDgUYutzWPP4KDomdDVMzogpz0EuG2HGrS09YWy34KiBJ9bP48TaWn5iosauGFzv+zE2fTF/YK6sxbOTOgU8mTteXwbHwYZSBrCLUj/dcAbrNCrE8IezxmUJR6W/byABg+tCSW1cVhjH9BrBp6uFgXESMY+tnUJeZcMTf+24hadsr0X4ivV1BXE20O2lZblesg6oxjvjnSsBNH2BDdZYMcX+4eEI3MN83Tesw"
const DOC = "<Document ID=\"_obj1\"><Data>Hello SAML</Data></Document>"
const DS = "http://www.w3.org/2000/09/xmldsig#"
const SHA256_DIGESTINFO = "3031300d060960864801650304020105000420"

fn sign_doc() {
    let key = RsaKey.private_from_pem(KEY_PEM)
    let k_octets = key["bits"] / 8
    let ref_canon = Xml.c14n_exclusive(DOC, {"id": "_obj1", "enveloped_signature": true})
    let digest_b64 = Base64.encode(Hex.decode(Crypto.sha256(ref_canon)))
    let si_inner = "<ds:SignedInfo>" +
      "<ds:CanonicalizationMethod Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"></ds:CanonicalizationMethod>" +
      "<ds:SignatureMethod Algorithm=\"http://www.w3.org/2001/04/xmldsig-more#rsa-sha256\"></ds:SignatureMethod>" +
      "<ds:Reference URI=\"#_obj1\"><ds:Transforms>" +
      "<ds:Transform Algorithm=\"http://www.w3.org/2000/09/xmldsig#enveloped-signature\"></ds:Transform>" +
      "<ds:Transform Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"></ds:Transform>" +
      "</ds:Transforms>" +
      "<ds:DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha256\"></ds:DigestMethod>" +
      "<ds:DigestValue>" + digest_b64 + "</ds:DigestValue></ds:Reference></ds:SignedInfo>"
    let si_standalone = "<ds:SignedInfo xmlns:ds=\"" + DS + "\">" + si_inner.substring(15, len(si_inner))
    let si_canon = Xml.c14n_exclusive(si_standalone)
    let si_hash = Crypto.sha256(si_canon)
    let em = Crypto.pkcs1_pad(SHA256_DIGESTINFO + si_hash, k_octets)
    let sig_hex = Crypto.modexp(em, key["d"], key["n"])
    let sig_b64 = Base64.encode(Hex.decode(sig_hex))
    let signature = "<ds:Signature xmlns:ds=\"" + DS + "\">" + si_inner +
      "<ds:SignatureValue>" + sig_b64 + "</ds:SignatureValue>" +
      "<ds:KeyInfo><ds:X509Data><ds:X509Certificate>" + CERT_B64 +
      "</ds:X509Certificate></ds:X509Data></ds:KeyInfo></ds:Signature>"
    let signed = DOC.replace("</Document>", signature + "</Document>")
    return {"signed": signed, "sig_hex": sig_hex, "si_hash": si_hash, "digest_b64": digest_b64}
}

describe("XML-DSig enveloped signing (Soli SP side)", fn() {
    test("the produced signature RSA-verifies against the public key", fn() {
        let r = sign_doc()
        let kp = X509.public_key(CERT_B64)
        let recovered = Crypto.pkcs1_unpad(Crypto.modexp(r["sig_hex"], kp["e"], kp["n"]))
        assert_eq(recovered, SHA256_DIGESTINFO + r["si_hash"]);
    });

    test("the signed document's Reference digest is correct (enveloped)", fn() {
        let r = sign_doc()
        let canon = Xml.c14n_exclusive(r["signed"], {"id": "_obj1", "enveloped_signature": true})
        assert_eq(Base64.encode(Hex.decode(Crypto.sha256(canon))), r["digest_b64"]);
    });

    test("SignedInfo re-extracted from the doc canonicalizes to what we signed", fn() {
        let r = sign_doc()
        let si = Xml.get_elements_by_tag(r["signed"], "SignedInfo")[0]
        assert_eq(Crypto.sha256(Xml.c14n_exclusive(si)), r["si_hash"]);
    });
});
