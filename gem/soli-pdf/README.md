# soli-pdf

Generate PDFs from JSON in Ruby, with the [Soli](https://github.com/solisoft/soli_lang) renderer.

## Installation

The gem lives in the Soli repository (not on RubyGems). In your `Gemfile`:

```ruby
gem 'soli-pdf', git: 'https://github.com/solisoft/soli_lang', glob: 'gem/soli-pdf/*.gemspec'
```

then `bundle install`.

## Usage

```ruby
require 'soli/pdf'

template = JSON.parse(File.read('invoice.json'))   # layout: a Hash, an Array or a JSON String
pdf = Soli::PDF.render(template: template, data: { 'invoice' => { 'number' => 'F-42' } })
File.binwrite('invoice.pdf', pdf)

Soli::PDF.render_to_file('invoice.pdf', template: template, data: data, title: 'Invoice F-42')
```

Options: `title`, `author`, `subject`, `password`, `owner_password`, `stationery:` (letterhead PDF path),
`attachments:` (paths), `fonts:` (extra font directories), `images: false` (do not fetch remote images),
`xml:` (Factur-X CII XML, with `profile:`), or `invoice:` instead of `data:` for a typed invoice.
Failures raise `Soli::PDF::RenderError` (with `stderr`); `Soli::PDF.last_warnings` lists skipped images and missing glyphs.

Images: `data:` URIs, files under the working directory, and `http(s)` URLs on public addresses
(`render_pdf` 2.9.1 and later; 2.9.0 drew `data:` URIs only).

Template reference: <https://soli-lang.org/docs/builtins/pdf>.

## The binary

On first use the gem downloads `render-pdf-<os>-<arch>.tar.gz` (the `render_pdf` executable plus
the bundled Titillium Web and JetBrains Mono fonts) from the Soli GitHub release matching
`Soli::PDF::BINARY_VERSION`, checks it against the published `.sha256`, and caches it in
`~/.cache/soli-pdf/`. To use your own build: `SOLI_PDF_BIN=/path/to/render_pdf` or `Soli::PDF.binary_path = ...`.

Release builds exist for Linux (amd64, arm64), macOS (arm64, amd64) and Windows (amd64). The Linux
binaries need **glibc 2.35 or later** (Ubuntu 22.04+, Debian 12+, RHEL/Rocky/Alma 10); on an older
system — RHEL/Rocky/Alma 9, Amazon Linux 2023, Debian 11, or Alpine (musl) — every render raises
`Soli::PDF::RenderError` with ``version `GLIBC_2.35' not found``. Build `render_pdf` on that system
(`cargo build --release --bin render_pdf` in `pdf/`, with `pdf/fonts` copied next to it) and point
`SOLI_PDF_BIN` at it.
