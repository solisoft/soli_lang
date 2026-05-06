# SEC-059: AWS credentials may surface in S3 error strings

- **Severity:** Medium
- **Status:** Todo
- **Location:** `src/interpreter/builtins/s3.rs:144, 162, 201-204, 240-243, 264-267, 302, 348`

**Issue:** Every S3 call wraps `RusotoError::Service` / `HttpDispatch` errors as `format!("Failed to ... {}", e)`. Some signing-failure modes include the request URL and signed headers (including the `Authorization: AWS4-HMAC-SHA256 ...` header). With dev mode on those errors flow into `http_log` and render in the dev bar — `Authorization` header content becomes visible to anyone seeing a screenshot.

**Fix:** Scrub `Authorization` and `x-amz-*` headers from error strings before bubbling up; log only the operation name and S3 error code.
