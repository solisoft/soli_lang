# EUI — images, lists and virtualisation

## Images and assets

```soli
avatar("public/images/avatar.png", 32)
```

The path is a file in the application, under `public/` or `app/assets/`.
Soli hashes it (BLAKE3), sends the hash in the tree, and serves the bytes at
`GET /_eui/asset/<hash>` with a one-year immutable cache header — the same
bytes for every session and every client, so a CDN can hold them and nothing
on the path can substitute them. The endpoint needs no session: anyone with
the hash gets the bytes, which is right for an image and why a path outside
those two directories — `config/`, `.env`, the publisher key — is refused
before it is read. A `src` is a server path; never build one from client
data.

## Keyed lists and virtualisation

Give repeated children a key (`keyed(id, row(...))`) and a re-sort becomes
`MoveChild` ops rather than a rebuild. A `list` with an item height is
virtualised on the client: it lays out only the rows it can see, so ten
thousand rows cost about what fifty do.

**Moving pictures.** `video(path, props, style, handlers)` puts a GIF or
an animated WebP in the tree (EUI spec 03 §8). It sizes itself to its
frames, `playing`, `loop` and `position` say what it should be doing, and
`ended` comes back. The client decodes it in its sandboxed worker and
advances it on its own clock, waking exactly when the next frame is due.

The client's **viewport** — width, height, scale, mode, density, font
scale — reaches the handler as `params["viewport"]` with the `connect`
event and again as a `viewport` event whenever it changes (a resize, a
mode switch). A view that keeps it in the state can lay itself out by
width: `examples/counter-app`'s music player collapses its sidebar to a
rail under 900 px and drops it under 640.

A `list` can be **windowed** (EUI spec 04 §7.1): give it `count`, the
number of rows, `heights`, one integer per row (or rely on `item_height`),
and a `window` handler; hand it only the children in view, each carrying
a `row` prop. The client lays out and scrolls the whole extent, asks
`window` with `[first, last]` when the rows in view change, and your
handler stores that range in the state so the next view builds those rows
and no other. A feed of forty thousand posts then costs the server one
window of cards. `examples/counter-app`'s `feed` is written this way
(`list_window` in its builders).

A keyed child is also what the server memoises. When the view returns, for
a keyed node, **the same hash object it returned last time**, Soli keeps the
converted subtree and skips it in the diff — it is neither walked nor
compared. So build repeated children once, keep them in a cache keyed by
whatever they depend on, and return the cached value; a change to one card
in a feed of ten thousand then costs one card, not ten thousand. The
contract is the usual one for a cache: a value handed back unchanged is
assumed unchanged, so never mutate a node hash after returning it — build
a new one instead. Unkeyed nodes and freshly built ones are converted and
diffed as before.

## Moving the scroll position

A node may carry `scroll_to` — a pair of pixel offsets — and the differ emits a
`ScrollTo` op for it. That is the one direction the protocol previously lacked: a
view could describe a scroll position but never move one, so "jump to the newest
message" had to be faked by rebuilding the list.

```soli
scroll({"scroll_to": [0, 99999]}, messages.map(fn(m) message_row(m)))
```

It is an instruction, not state: the client scrolls when the op arrives, and the
viewer may scroll away again afterwards.