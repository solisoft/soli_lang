# Third-party assets

The only asset embedded in the binary is the ICC profile. Fonts are **not**
embedded — they are loaded at runtime from a font directory (see README).

## Fonts (SIL Open Font License 1.1) — runtime assets, not embedded

Shipped on disk for local dev/tests (in `fonts/`) and for the lang app
(`../lang/font/`); the library loads them at runtime via `font_dirs`.

- **Titillium Web** (`fonts/TitilliumWeb-Regular.ttf`, `TitilliumWeb-Bold.ttf`,
  `TitilliumWeb-Italic.ttf`, `TitilliumWeb-BoldItalic.ttf`)
  © Accademia di Belle Arti di Urbino and students of MA course of Visual
  Design. Licensed under SIL OFL 1.1.
  <https://fonts.google.com/specimen/Titillium+Web>

- **JetBrains Mono** (`fonts/JetBrainsMono-Regular.ttf`, `-Bold.ttf`, `-Italic.ttf`)
  — the monospace face for inline `code` spans. © The JetBrains Mono Project
  Authors. Licensed under SIL OFL 1.1.
  <https://github.com/JetBrains/JetBrainsMono>

- **Noto Sans JP** (`../lang/font/NotoSansJP-Regular.ttf`) — CJK / full-Unicode
  fallback. A static `wght=400` instance produced from the Google Fonts variable
  font. © The Noto Project Authors. Licensed under SIL OFL 1.1.
  <https://fonts.google.com/noto/specimen/Noto+Sans+JP>

The SIL OFL 1.1 permits bundling, embedding, and redistribution. Full license
text: <https://openfontlicense.org/>.

## Code dependencies of note

- **qrcode** (crates.io `qrcode` 0.14, MIT OR Apache-2.0) — used to encode the
  payment QR. We use it for the module matrix only and rasterise it ourselves, so
  its optional `image`/`svg` renderers are disabled (`default-features = false`).
  The EPC "GiroCode" payload follows the **EPC069-12** "Quick Response Code:
  Guidelines to Enable Data Capture for the Initiation of a SEPA Credit Transfer"
  specification (European Payments Council). No code from that spec is bundled.

## Vendored crates

- **printpdf** (`vendor/printpdf/`, MIT) — a byte-for-byte copy of printpdf
  0.9.1 from crates.io © Felix Schütt, Julien Schminke. The **only** change to
  the source is in `vendor/printpdf/Cargo.toml`: its internal `lopdf` pin is
  bumped from `0.39.0` to `0.43` to fix **RUSTSEC-2026-0187** (a stack overflow
  when parsing deeply-nested PDFs). Upstream 0.9.1 — the only published release,
  and current `master` — pins `lopdf ^0.39`, so a crates.io dependency cannot be
  patched across the caret; vendoring is the only way to ship the fixed lopdf.
  Delete the vendor directory and restore the crates.io dependency once upstream
  printpdf publishes a release built on `lopdf >= 0.42`.
  Its bundled assets keep their own licenses: the default PDF base-14 font
  subsets (`vendor/printpdf/defaultfonts/`) and the `CoatedFOGRA39.icc` profile
  (`vendor/printpdf/src/res/`, see the adjacent `.icc.LICENSE.txt`).
  <https://github.com/fschutt/printpdf>

## ICC color profile (CC0 / public domain)

- **sRGB v2 micro** (`assets/sRGB-v2-micro.icc`) from the saucecontrol
  Compact-ICC-Profiles project, released under CC0 1.0 (public domain).
  Used as the PDF/A-3 OutputIntent destination profile.
  <https://github.com/saucecontrol/Compact-ICC-Profiles>
