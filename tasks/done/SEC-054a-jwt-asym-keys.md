# SEC-054a: JWT only supports HS256/384/512 (no RS256 / Ed25519)

- **Severity:** Low (feature gap rather than regression)
- **Status:** Todo
- **Location:** `src/interpreter/builtins/jwt.rs` (algorithm parsing in `jwt_sign`, `Validation::default()` in `jwt_verify`)

**Issue:** Only HMAC family is wired in. Production deployments needing asymmetric verification (e.g. issuing JWTs from one service and verifying in many) are forced to share a symmetric secret across services. `Validation::default()` also limits verify to HS256 — using HS384/512 needs an explicit `validation.algorithms = vec![...]`.

**Fix:** Add `RS256` and `EdDSA` (Ed25519) signing/verification paths. Accept PEM-encoded private/public keys via the `secret` arg (or a new `key` opt). Configure `Validation::algorithms` to match the algorithm in the JWT header so HS{256,384,512} verification works without manual setup.

Spun off from SEC-054 — that task closed the immediate weakness (16 → 32 byte minimum + high-entropy guidance). This is a feature addition, not a regression closer; queued behind it.
