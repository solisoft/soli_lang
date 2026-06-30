# Third-party assets

The only asset embedded in the binary is the ICC profile. Fonts are **not**
embedded — they are loaded at runtime from a font directory (see README).

## Fonts (SIL Open Font License 1.1) — runtime assets, not embedded

Shipped on disk for local dev/tests (in `fonts/`) and for the lang app
(`../lang/font/`); the library loads them at runtime via `font_dirs`.

- **Titillium Web** (`fonts/TitilliumWeb-Regular.ttf`, `TitilliumWeb-Bold.ttf`)
  © Accademia di Belle Arti di Urbino and students of MA course of Visual
  Design. Licensed under SIL OFL 1.1.
  <https://fonts.google.com/specimen/Titillium+Web>

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

## ICC color profile (CC0 / public domain)

- **sRGB v2 micro** (`assets/sRGB-v2-micro.icc`) from the saucecontrol
  Compact-ICC-Profiles project, released under CC0 1.0 (public domain).
  Used as the PDF/A-3 OutputIntent destination profile.
  <https://github.com/saucecontrol/Compact-ICC-Profiles>
