# Pocket Garden

A small gardening game in a native EUI window, written as a plain Soli script:
plant, water and harvest in real time, buy seeds, press your crops into a
herbarium. The tutorial that builds it step by step is on the blog:
[Pocket Garden: a Little Game in a Native Window, from One Script](https://soli.solisoft.net/docs/blog/pocket-garden-eui).

| File | What it holds |
|---|---|
| `garden.sl` | `eui_window`, the handler, the header and the tabs — the script you run |
| `species.sl` | the four plants |
| `beds.sl` | the rules (`Garden`), the canvas drawings, the garden screen |
| `shop.sl` | the seed shop |
| `herbarium.sl` | the harvest chart |
| `icon.svg`, `icon.png` | the app icon |

## Play

```bash
soli garden.sl
```

The window needs a `soli` built with the `eui-desktop` feature
(`cargo install --path . --locked --features eui-desktop`); the published
binaries leave it out.

## Ship it

```bash
soli build garden.sl -o pocket-garden
```

writes one executable that carries the game and the runtime that built it. Copy
it anywhere and run it; no `soli` and no source files are needed there. Build it
with an `eui-desktop` soli, on the platform you ship to: `--target` uses a
prebuilt runtime without the window.

For macOS, `.github/workflows/pocket-garden-macos.yml` does the whole thing on
an Apple Silicon runner and uploads `PocketGarden.dmg`: the executable in
`Pocket Garden.app`, with this icon, ad-hoc signed. It is not notarized, so the
first launch asks for confirmation: Control-click the app, choose **Open**, then
**Open** again.
