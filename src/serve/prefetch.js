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

    // Use `<link rel="prefetch">` (no `as` attribute). The browser fetches the
    // URL at low priority and stores it in the **prefetched-resources cache**,
    // which navigations consume *before* checking the HTTP cache. This is the
    // mechanism `<link rel="prefetch">` is designed for and is how Turbo Drive,
    // instant.page, and Quicklink all warm navigations.
    //
    // Why not `fetch()`: a `fetch()` response goes into the HTTP cache with
    // `Sec-Fetch-Mode: cors`/`no-cors`. The subsequent navigation has
    // `Sec-Fetch-Mode: navigate` — cache partitioning treats these as different
    // requests and the prefetch doesn't reuse on click.
    //
    // Why not `as="document"`: that routes into Chromium's stricter
    // document-prefetch cache with extra reuse conditions (COEP, etc.); plain
    // `rel="prefetch"` is simpler and works in Chromium, Firefox, and Safari.
    function prefetch(url) {
        seen.add(url);
        try {
            var link = document.createElement("link");
            link.rel = "prefetch";
            link.href = url;
            // Don't set `as` — let the browser treat it as a generic prefetch
            // that any future navigation can consume.
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
