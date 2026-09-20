# Character encodings

Soli strings are UTF-8. `Encoding` converts between UTF-8 and the legacy byte
encodings — Latin-1 / ISO-8859-1 / Windows-1252 and friends — so a non-UTF-8
file can be imported without turning every accented character into `?`.

A byte like `0xE9` (`é` in Latin-1) is not valid UTF-8 on its own, so reading a
Latin-1 file as text either fails or garbles the accents. Decode the raw bytes
from the charset they are actually in.

This comes up for:

- CSV and fixed-width exports from older systems
- HTTP responses declared `charset=ISO-8859-1`
- Writing a file back out in the charset a downstream system expects

Labels follow the [WHATWG Encoding Standard](https://encoding.spec.whatwg.org/)
— `"latin1"`, `"iso-8859-1"`, `"windows-1252"`, `"utf-8"`, and so on. Note that
`latin1` and `iso-8859-1` both alias to `windows-1252`, which is what browsers
do and what the byte streams in the wild usually mean. An unknown label raises.

## `Encoding.decode(input, label)`

Decodes `input` — a byte array (`Array<Int>`) or a string — from `label` into a
UTF-8 string.

```soli
# café in Latin-1: c=99 a=97 f=102 é=233
Encoding.decode([99, 97, 102, 233], "latin1")   # "café"
Encoding.decode([233], "iso-8859-1")            # "é"
```

## `Encoding.encode(text, label)`

Encodes a UTF-8 string into a byte array in `label`. A character the target
charset cannot represent becomes an HTML numeric entity — an emoji encodes as
`&#128512;` rather than silently disappearing.

```soli
Encoding.encode("café", "latin1")   # [99, 97, 102, 233]

text  = "Curaçao — déjà vu"
bytes = Encoding.encode(text, "windows-1252")
assert_eq(Encoding.decode(bytes, "windows-1252"), text)
```

## Reading and writing files

`slurp` and `File.read` take a charset label directly, so an import is one line:

```soli
text = slurp("clients.csv", "latin1")
text = File.read("clients.csv", "latin1")

# or explicitly, through the raw bytes
raw  = slurp("clients.csv", "binary")
text = Encoding.decode(raw, "latin1")
```

`barf` and `File.write` take a byte array, so pair them with `Encoding.encode`:

```soli
barf("clients.csv", Encoding.encode(text, "latin1"))
```

## Failure

An unrecognised label raises, like any other error:

```soli
try
  Encoding.decode(bytes, "no-such-encoding")
catch e
  print("Error: " + e)   # unknown encoding: no-such-encoding
end
```

## See also

- [`base64.md`](base64.md) — Base64, standard and URL-safe
- Rendered page: `/docs/utility/encoding`
