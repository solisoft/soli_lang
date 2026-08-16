# Encrypted attributes (`encrypts`): AES-256-GCM at rest, plaintext in memory.
# Query-shape assertions are not needed; persistence is gated on a working
# database *and* SOLI_ENCRYPTION_KEY (create raises without the key).

class EncUser < Model
  encrypts(:ssn)
end

let __enc_available = false
try
  let probe = EncUser.create({ "email": "probe@x.co", "ssn": "000-00-0000" })
  if !probe.nil? && probe._errors.nil?
    __enc_available = true
    probe.delete()
  end
catch e
  __enc_available = false
end

describe("encrypts", fn() {
  test("create and find return plaintext", fn() {
    return unless __enc_available

    let user = EncUser.create({ "email": "a@x.co", "ssn": "123-45-6789" })
    assert(user._errors.nil?)
    assert_eq(user.ssn, "123-45-6789")
    assert_eq(EncUser.find(user._key).ssn, "123-45-6789")
    user.delete()
  })

  test("save updates the ciphertext and still decrypts", fn() {
    return unless __enc_available

    let user = EncUser.create({ "email": "b@x.co", "ssn": "111-11-1111" })
    user.ssn = "222-22-2222"
    assert(user.save())
    assert_eq(user.ssn, "222-22-2222")
    assert_eq(EncUser.find(user._key).ssn, "222-22-2222")
    user.delete()
  })

  test("plaintext equality does not match stored ciphertext", fn() {
    return unless __enc_available

    let user = EncUser.create({ "email": "c@x.co", "ssn": "333-33-3333" })
    # Hash where is portable; it compares the stored ciphertext, which never
    # equals the plaintext (AES-GCM uses a random nonce).
    assert_eq(EncUser.where({ "ssn": "333-33-3333" }).count(), 0)
    assert_eq(EncUser.where({ "email": "c@x.co" }).count(), 1)
    user.delete()
  })

})
