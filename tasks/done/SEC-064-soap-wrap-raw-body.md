# SEC-064: `SOAP.wrap` interpolates body raw

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/interpreter/builtins/soap.rs:175-185`

**Issue:** `SOAP.wrap(body)` does not escape `body`, so a Soli developer who passes user input there gets XML injection. By design (callers must pre-escape with `SOAP.xml_escape`), but easy to misuse.

**Fix:** Add a docstring warning + an opt-in `escape: true` flag.
