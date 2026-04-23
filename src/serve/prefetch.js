// Soli hover-prefetch. Embedded verbatim in the soli binary; served as an
// external script at /__soli/prefetch.js so strict CSP (no unsafe-inline) works.
// Auto-injected into every HTML response unless SOLI_PREFETCH=off.
// Per-link opt-out: <a data-no-prefetch> or any ancestor with that attribute.
(function () {
    if (window.__soliPrefetchInstalled) return;
    window.__soliPrefetchInstalled = true;

    var DELAY = 65;   // ms — avoid prefetching fly-over hovers
    var seen = new Set();

    var conn = navigator.connection;
    if (conn && (conn.saveData || /2g/.test(conn.effectiveType || ""))) return;

    function shouldPrefetch(a) {
        if (!a.href) return false;
        if (a.origin !== location.origin) return false;
        // Skip self-links — there's no navigation to accelerate. Ignore the
        // hash so `<a href="/foo#bar">` on `/foo` still counts as self.
        if (a.pathname === location.pathname && a.search === location.search) return false;
        if (a.protocol !== "http:" && a.protocol !== "https:") return false;
        if (a.hasAttribute("data-no-prefetch")) return false;
        if (a.closest("[data-no-prefetch]")) return false;
        var method = (a.getAttribute("data-method") || "get").toLowerCase();
        if (method !== "get") return false;
        if (seen.has(a.href)) return false;
        return true;
    }

    // Use `<link rel="prefetch" as="document">`. The browser fetches the URL
    // at low priority and stores it in the **document-prefetch cache**, which
    // top-level navigations consume before hitting the HTTP cache. This is
    // the only form Chromium actually promotes to a navigation response in
    // current versions — bare `rel="prefetch"` (no `as`) lands in a
    // subresource cache that navigations skip, so the prefetched body is
    // never reused.
    //
    // Why not `fetch()`: a `fetch()` response goes into the HTTP cache with
    // `Sec-Fetch-Mode: cors`/`no-cors`. The subsequent navigation has
    // `Sec-Fetch-Mode: navigate` — cache partitioning treats these as
    // different requests and the prefetch doesn't reuse on click.
    //
    // Reuse semantics at click time:
    //   - Fresh entry (`max-age > 0` and within TTL) → served directly.
    //   - Stale or `no-cache` → browser issues a conditional GET with
    //     `If-None-Match` against the prefetched body's ETag. Server can
    //     respond 304 and the prefetched bytes get promoted as the nav
    //     response — which is exactly what `html_response` now emits.
    function prefetch(url) {
        seen.add(url);
        try {
            var link = document.createElement("link");
            link.rel = "prefetch";
            link.as = "document";
            link.href = url;
            document.head.appendChild(link);
        } catch (e) { /* ignore */ }
    }

    var timer = null;

    document.addEventListener("mouseover", function (e) {
        var a = e.target.closest && e.target.closest("a");
        if (!a || !shouldPrefetch(a)) return;
        if (timer) clearTimeout(timer);
        timer = setTimeout(function () { prefetch(a.href); }, DELAY);
    }, { passive: true });

    document.addEventListener("mouseout", function () {
        if (timer) { clearTimeout(timer); timer = null; }
    }, { passive: true });

    // Touch: no hover, but touchstart is a strong intent signal.
    document.addEventListener("touchstart", function (e) {
        var a = e.target.closest && e.target.closest("a");
        if (a && shouldPrefetch(a)) prefetch(a.href);
    }, { passive: true });
})();
