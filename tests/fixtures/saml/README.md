# SAML test fixtures

Two throwaway RSA-2048 key pairs with self-signed certificates, for
`src/interpreter/builtins/saml.rs` and `tests/saml_generator_test.rs`: `idp` is
the identity provider the tests trust, `other` signs the forgeries. They
protect nothing — never configure either certificate in an application.

    openssl req -x509 -newkey rsa:2048 -nodes -keyout idp.key -out idp.crt \
      -days 36500 -subj "/CN=Test IdP idp"

`weak.crt` is an RSA-1024 certificate (its key was not kept), for the test
that refuses a signing key under 2048 bits:

    openssl req -x509 -newkey rsa:1024 -nodes -keyout /dev/null -out weak.crt \
      -days 36500 -subj "/CN=weak-idp.test"
