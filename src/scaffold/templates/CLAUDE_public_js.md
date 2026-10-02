# public/js — client scripts under instant navigation

Every page is served with `/__soli/nav.js`. A same-origin link click fetches
the next page and **swaps `<body>` in place**; the document never reloads. The
consequence for any file in this directory:

- **An external `<script src>` runs once per tab, not once per page.** Code
  that runs at the top level, or on `DOMContentLoaded`, sets up the page the
  visitor *landed on* and never runs again. After a link click, widgets on the
  new page are dead: no animation, no copy button, no highlighted code.
- **What the previous page started keeps running.** `requestAnimationFrame`
  loops, `setInterval`s, `IntersectionObserver`s and `document`/`window`
  listeners survive the swap, now attached to detached nodes, and stack up
  with every visit.

## The pattern

```js
(function () {
  var stopCurrent = null;            // teardown for what the last page started

  function init() {
    // 1. Idempotent: skip what is already set up. nav.js does not fire
    //    soli:load on the first full load, but don't rely on that.
    document.querySelectorAll('.widget:not([data-ready])').forEach(function (el) {
      el.dataset.ready = '';
      setUp(el);
    });

    // 2. Long-lived work: stop the previous page's, keep a stop() for yours.
    var canvas = document.getElementById('scene');
    if (canvas && canvas.hasAttribute('data-ready')) return;  // same page, already running
    if (stopCurrent) { stopCurrent(); stopCurrent = null; }
    if (!canvas) return;
    canvas.setAttribute('data-ready', '');

    var running = true;
    function onVisibility() { running = !document.hidden; if (running) requestAnimationFrame(frame); }
    function frame() {
      if (!running) return;
      if (!canvas.isConnected) { stop(); return; }   // swapped out mid-flight
      draw();
      requestAnimationFrame(frame);
    }
    function stop() {
      running = false;
      document.removeEventListener('visibilitychange', onVisibility);
    }
    document.addEventListener('visibilitychange', onVisibility);
    requestAnimationFrame(frame);
    stopCurrent = stop;
  }

  init();                                         // the page you landed on
  document.addEventListener('soli:load', init);   // every page after a swap
})();
```

Inline `<script>` blocks in a view are different: they re-run on every visit,
and `DOMContentLoaded` listeners they register are replayed, so page-specific
inline code needs none of this.

## Other tools

- A live widget that should **survive** navigation (map, video, editor): give
  it an `id` and `data-soli-permanent` on both pages instead of re-creating it.
- Opt a link or container out with `data-no-nav`, a page with
  `<meta name="soli-nav" content="off">`.
- Lifecycle events on `document`: `soli:visit` (before a visit, cancelable),
  `soli:before-render` (cancelable), `soli:load` (after every swap).

## Verify it with a link click, not a reload

A full reload always runs your script, so it proves nothing. Open a page that
does **not** have the widget, then click a link to one that does; then
navigate away and come back (and try the back button). The widget must work
each time, and only one copy of it may be running.

Full reference: `docs/views.md` → "Instant Navigation".
