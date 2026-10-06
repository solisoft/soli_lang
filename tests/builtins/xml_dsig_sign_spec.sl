# XML-DSig enveloped signing on the SP side: Soli builds and signs a SAML-style
# Signature from RsaKey, exclusive C14N, modexp and PKCS#1, then checks that
# the result verifies the way a relying party would verify it.

const KEY_PEM = "-----BEGIN PRIVATE KEY-----\n" +
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
const CERT_B64 = "MIICqzCCAZOgAwIBAgIBATANBgkqhkiG9w0BAQsFADAZMRcwFQYDVQQDDA5zcC5leGFtcGxlLmNvbTAeFw0" +
  "yMDAxMDEwMDAwMDBaFw0zNTAxMDEwMDAwMDBaMBkxFzAVBgNVBAMMDnNwLmV4YW1wbGUuY29tMIIBIjANBgkqhkiG9w0BAQEFAAO" +
  "CAQ8AMIIBCgKCAQEAnGv2TK9t8LCDPS2OZz3FeFYkMXUN9AmuuRWTAmV0GEMvnnQTHl3lRd+A81SkmVJMgCmfd1OkoWVZG5ML6hS" +
  "T9Y2K8zFPqL9nnpqWINDfVgxtNjWjMOuWVWih20eTeGsRx8Ri6XWW8FTz5bACJuwNWDJOHxeILZ3hA+I417QlWMM05Adh1noAGKq" +
  "UpLs3sqeBf+ixQjzkOVQ6oGUTUlwmVUAuNg5rdEDDSAqsio3H0wfyAEtQ7UWMKgXwAzT0vjhpwEXeYlOtIAQgiwQ9/Bt2QwCPz9x" +
  "7KJ16X6O8PurDJ9gfFfFT73fTt9qeTwfja5iHCCaaHIqSJ4DNCzjy2gvHjQIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQBUHvn9HJE" +
  "Mkw4NZ95aSbmNAF+uMoCOCuj9NotJrmbUFdjLpfSvWpx4lSI6ZoUdXa7QyN/Wi7gBJgSl1ex6axSAD3Al5fEZqBuEqsMMncRGf2y" +
  "463MGDgUYutzWPP4KDomdDVMzogpz0EuG2HGrS09YWy34KiBJ9bP48TaWn5iosauGFzv+zE2fTF/YK6sxbOTOgU8mTteXwbHwYZS" +
  "BrCLUj/dcAbrNCrE8IezxmUJR6W/byABg+tCSW1cVhjH9BrBp6uFgXESMY+tnUJeZcMTf+24hadsr0X4ivV1BXE20O2lZblesg6o" +
  "xjvjnSsBNH2BDdZYMcX+4eEI3MN83Tesw"
const DOC = "<Document ID=\"_obj1\"><Data>Hello SAML</Data></Document>"
const DS = "http://www.w3.org/2000/09/xmldsig#"
const SHA256_DIGESTINFO = "3031300d060960864801650304020105000420"

# Build an enveloped RSA-SHA256 signature over DOC and return the signed
# document with the intermediate values a verifier recomputes.
def sign_doc
  key = RsaKey.private_from_pem(KEY_PEM)
  reference_c14n = Xml.c14n_exclusive(DOC, {"id": "_obj1", "enveloped_signature": true})
  digest_b64 = Base64.encode(Hex.decode(Crypto.sha256(reference_c14n)))
  signed_info_body = "<ds:CanonicalizationMethod Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\">" +
    "</ds:CanonicalizationMethod>" +
    "<ds:SignatureMethod Algorithm=\"http://www.w3.org/2001/04/xmldsig-more#rsa-sha256\"></ds:SignatureMethod>" +
    "<ds:Reference URI=\"#_obj1\"><ds:Transforms>" +
    "<ds:Transform Algorithm=\"http://www.w3.org/2000/09/xmldsig#enveloped-signature\"></ds:Transform>" +
    "<ds:Transform Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"></ds:Transform>" +
    "</ds:Transforms>" +
    "<ds:DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha256\"></ds:DigestMethod>" +
    "<ds:DigestValue>#{digest_b64}</ds:DigestValue></ds:Reference></ds:SignedInfo>"
  # SignedInfo is canonicalized on its own, so it carries the ds namespace itself
  standalone_signed_info = "<ds:SignedInfo xmlns:ds=\"#{DS}\">" + signed_info_body
  signed_info_hash = Crypto.sha256(Xml.c14n_exclusive(standalone_signed_info))
  encoded_message = Crypto.pkcs1_pad(SHA256_DIGESTINFO + signed_info_hash, key["bits"] / 8)
  signature_hex = Crypto.modexp(encoded_message, key["d"], key["n"])
  signature = "<ds:Signature xmlns:ds=\"#{DS}\"><ds:SignedInfo>" + signed_info_body +
    "<ds:SignatureValue>" + Base64.encode(Hex.decode(signature_hex)) + "</ds:SignatureValue>" +
    "<ds:KeyInfo><ds:X509Data><ds:X509Certificate>" + CERT_B64 +
    "</ds:X509Certificate></ds:X509Data></ds:KeyInfo></ds:Signature>"
  {
    "signed": DOC.replace("</Document>", signature + "</Document>"),
    "signature_hex": signature_hex,
    "signed_info_hash": signed_info_hash,
    "digest_b64": digest_b64
  }
end

signing = nil

describe("XML-DSig enveloped signing (Soli SP side)") do
  before_each() do
    signing = sign_doc()
  end

  test("the produced signature RSA-verifies against the certificate's public key") do
    public_key = X509.public_key(CERT_B64)
    recovered = Crypto.pkcs1_unpad(Crypto.modexp(signing["signature_hex"], public_key["e"], public_key["n"]))
    assert_eq(recovered, SHA256_DIGESTINFO + signing["signed_info_hash"])
  end

  test("PKCS#1 v1.5 signing is deterministic") do
    assert_eq(sign_doc()["signature_hex"], signing["signature_hex"])
  end

  test("the signed document's Reference digest is correct (enveloped)") do
    canonical = Xml.c14n_exclusive(signing["signed"], {"id": "_obj1", "enveloped_signature": true})
    assert_eq(Base64.encode(Hex.decode(Crypto.sha256(canonical))), signing["digest_b64"])
  end

  test("SignedInfo re-extracted from the document canonicalizes to what was signed") do
    signed_info = Xml.get_elements_by_tag(signing["signed"], "SignedInfo")[0]
    assert_eq(Crypto.sha256(Xml.c14n_exclusive(signed_info)), signing["signed_info_hash"])
  end

  test("the signature is enveloped inside the document, after the data") do
    assert(signing["signed"].starts_with("<Document ID=\"_obj1\"><Data>Hello SAML</Data><ds:Signature "))
    assert(signing["signed"].ends_with("</ds:Signature></Document>"))
    assert_contains(signing["signed"], "<ds:X509Certificate>#{CERT_B64}</ds:X509Certificate>")
  end
end
