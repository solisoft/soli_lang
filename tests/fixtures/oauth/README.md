# OAuth test fixtures

`apple_test_key.p8` / `.pub`: a throwaway P-256 key pair, in the form Apple
hands out a "Sign in with Apple" key. It signs the client secret in
`tests/oauth_generator_test.rs` and the ES256 cases of
`tests/builtins/jwt_spec.sl`. It protects nothing.

    openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out apple_test_key.p8
    openssl pkey -in apple_test_key.p8 -pubout -out apple_test_key.pub
