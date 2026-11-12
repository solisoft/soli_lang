# Passkeys end to end: the page's navigator.credentials calls answered by
# Chrome's virtual authenticator, the server verifying with WebAuthn.*.
#
# Run with: soli test tests-e2e/browser/specs --browser

# WebAuthn refuses an IP address as the relying party: same server, by name.
def visit_passkeys_on_localhost()
  visit("/passkeys")
  visit(page_url().replace("127.0.0.1", "localhost"))
end

describe("passkeys with a virtual authenticator") do
  test("registers a passkey, then signs in with it") do
    visit_passkeys_on_localhost()
    authenticator = virtual_authenticator()
    click_button("Register")
    wait_for_text("registered alg -7 none uv true")

    credentials = virtual_credentials(authenticator)
    assert_eq(credentials.length, 1)
    assert_eq(credentials[0]["rp_id"], "localhost")
    assert_eq(credentials[0]["user_handle"], "user-1")
    assert(credentials[0]["resident"])

    click_button("Sign in")
    wait_for_text("signed in as user-1")
    # The server stored the counter the authenticator now holds.
    count = virtual_credentials(authenticator)[0]["sign_count"]
    assert(count > 0)
    assert_text("signed in as user-1 count #{count}")
  end

  test("refuses a tampered assertion") do
    visit_passkeys_on_localhost()
    virtual_authenticator()
    click_button("Register")
    wait_for_text("registered")
    click_button("Sign in (tampered)")
    wait_for_text("refused: WebAuthn.verify_authentication(): signature is invalid")
  end

  # (A USB key without user verification is refused by Chrome itself here:
  # the options ask for a discoverable credential, which it will not create
  # on a key that cannot verify the user.)
  test("an authenticator that cannot verify the user registers, unverified") do
    visit_passkeys_on_localhost()
    virtual_authenticator({"user_verification": false})
    click_button("Register")
    wait_for_text("registered alg -7 none uv false")
  end

  test("each test starts without the previous test's authenticator") do
    visit_passkeys_on_localhost()
    authenticator = virtual_authenticator()
    assert_eq(virtual_credentials(authenticator), [])
    remove_virtual_authenticator(authenticator)
  end

  test("refuses an unknown option or transport") do
    assert_raises("unknown option \"colour\"") do
      virtual_authenticator({"colour": "red"})
    end
    assert_raises("transport is one of") do
      virtual_authenticator({"transport": "carrier-pigeon"})
    end
  end
end
