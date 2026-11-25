# Bytes, Path and Zip

Three classes for values the rest of the framework already moves around as strings. None of them touch the disk.

## Bytes

`Bytes` is one binary value. PDF, images, uploads and HTTP bodies stay base64 strings; `Bytes` does not change those return types. The payload is stored on the instance. `to_string` refuses bytes that are not UTF-8.

```soli
hello = Bytes.from_string("hello")
hello.length                  # 5
hello.to_base64               # "aGVsbG8="
hello.slice(1, 4).to_string   # "ell" — end is exclusive, like a Soli range

raw = Bytes.from_array([104, 105])
raw.to_string                 # "hi"

again = Bytes.from_base64("aGVsbG8=")
again.to_string               # "hello"
```

`from_base64` takes standard base64. `from_array` takes integers in `0..255`. `slice(start, end?)` defaults `end` to the length. A range past the value, or an end before the start, or a negative index, is an error. `slice(5, 5)` on a 5-byte value is empty.

## Path

`Path` joins and splits a slash-separated string. It does not read or write files, and it does not normalize `.` or `..`. Separators are `/` on every platform. A later absolute part does not reset the prefix. A leading slash stays only when the first part starts with `/`. Empty segments from doubled slashes are dropped.

```soli
Path.join("app", "models", "post.sl")   # "app/models/post.sl"
Path.join("/var/", "/app")              # "/var/app"
Path.join("a//b")                       # "a/b"

Path.dirname("app/models/post.sl")      # "app/models"
Path.dirname("post.sl")                 # "."
Path.dirname("/a")                      # "/"
Path.dirname("a//b")                    # "a"
Path.basename("/")                      # "/"
Path.extname("archive.tar.gz")          # ".gz"
Path.extname(".gitignore")              # ""
```

`dirname`, `basename` and `extname` trim trailing slashes first. `extname` includes the dot and uses the last one. A name whose only dot is leading has no extension, and neither has `.` or `..`. Slashes doubled before the last name go with the separator: `Path.dirname("a//b")` is `"a"`, as `Path.join` reads it.

## Zip

`Zip` lists, reads and writes a zip archive in memory. Inputs and outputs are base64, the same shape as `PDF`. `Zip.create` takes a hash of entry name to contents: a string is UTF-8, `{"base64": "..."}` is raw bytes, and an array of integers `0..255` is raw bytes too. `Zip.entries` returns `{name, size, compressed_size, directory}` for each entry. `Zip.read` returns the entry as base64. `Zip.read_text` returns UTF-8, or an error.

```soli
archive = Zip.create({
  "hello.txt": "hi",
  "bin.dat": {"base64": "YWJj"}
})
Zip.read_text(archive, "hello.txt")   # "hi"
Zip.read(archive, "bin.dat")           # "YWJj"
Zip.entries(archive).length            # 2

Zip.read(upload, "scan.tiff", {"max_size": 200 * 1024 * 1024})   # a larger entry
```

`Zip.read` and `Zip.read_text` inflate at most `max_size` bytes of one entry, 32 MiB unless the call passes `{"max_size": n}`. A few kilobytes of upload can declare, or inflate to, gigabytes, so an entry past the cap is an error, not a partial read. They take any name `Zip.entries` lists, as written in the archive (`./readme.txt`, `docs\guide.txt`). A name the archive does not hold is `has no entry`; an entry that cannot be opened (encrypted, an unsupported compression) says why.

Names given to `Zip.create` must be relative. An empty name, a leading slash, a backslash, or an empty, `.` or `..` segment is refused. The archive is not written to disk. A size that does not fit an integer is an error. `Zip` is part of the `office` feature, which is on by default.
