# Base64

Encode and decode data as Base64 — for binary that has to travel through
something text-only: a JSON payload, a URL, an HTTP header, a data URL.

Two alphabets, and the difference matters:

| | Alphabet | Padding | Use it for |
|---|---|---|---|
| `Base64.encode` / `decode` | standard, with `+` and `/` (RFC 4648 §4) | `=` | JSON payloads, Basic auth, data URLs |
| `Base64.urlsafe_encode` / `urlsafe_decode` | `-` and `_` (RFC 4648 §5) | none on encode; tolerated on decode | JWS, JWK, PKCE, JWK thumbprints |

## Standard

```soli
encoded = Base64.encode("Hello, World!")
# "SGVsbG8sIFdvcmxkIQ=="

Base64.encode("123")      # "MTIz"
Base64.encode("\n\t")     # "CQo="

original   = "Hello, World!"
round_trip = Base64.decode(Base64.encode(original))
assert_eq(round_trip, original)
```

`decode` returns a `String` when the bytes are valid UTF-8, and an array of byte
integers when they are not — so binary survives a round trip without being
mangled into replacement characters.

## URL-safe

`urlsafe_encode` never pads. That is not an omission: JWS, JWK, PKCE and JWK
thumbprints all specify unpadded output, so there is deliberately no option to
add it back.

```soli
# Safe to drop straight into a URL or a JWT segment
Base64.urlsafe_encode("Hello, World!")   # "SGVsbG8sIFdvcmxkIQ"

# PKCE S256 challenge. `Crypto.sha256` returns hex, so decode to raw bytes
# first — encoding the hex *text* yields a different, wrong value.
challenge = Base64.urlsafe_encode(Hex.decode(Crypto.sha256(code_verifier)))
```

`urlsafe_decode` accepts padding or its absence, because producers in the wild
disagree about stripping `=`:

```soli
Base64.urlsafe_decode("SGVsbG8sIFdvcmxkIQ")     # "Hello, World!"
Base64.urlsafe_decode("SGVsbG8sIFdvcmxkIQ==")   # same — padding tolerated

# Reading a JWT header without verifying it
header = JSON.parse(Base64.urlsafe_decode(token.split(".")[0]))
```

## Common shapes

**Binary in a JSON payload**

```soli
image_data   = slurp("avatar.png")
base64_image = Base64.encode(image_data)
json_response = json_stringify({
  "avatar": base64_image,
  "filename": "avatar.png"
})
```

**Basic auth**

```soli
def basic_auth_header(username: String, password: String) -> String
  "Basic " + Base64.encode(username + ":" + password)
end

basic_auth_header("user", "secret123")
# "Basic dXNlcjpzZWNyZXQxMjM="
```

**A data URL**

```soli
svg_content = slurp("icon.svg")
data_url = "data:image/svg+xml;base64," + Base64.encode(svg_content)
```

## Failure

`Base64.decode` raises when the input carries characters outside the alphabet,
or when its padding is wrong.

```soli
try
  Base64.decode("invalid==")
catch e
  print("Error: " + e)
end

# Or, with a fallback
def safe_decode(input: String) -> String
  Base64.decode(input) rescue ""
end
```

## See also

- [`encoding.md`](encoding.md) — Hex, URL and HTML encoding
- Rendered page: `/docs/utility/base64`
