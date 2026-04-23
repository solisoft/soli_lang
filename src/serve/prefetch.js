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

    // Use `fetch()` instead of `<link rel="prefetch">`. Same-origin `fetch`
    // runs at normal priority (higher than prefetch) and populates the HTTP
    // cache reliably — which is what the browser consults on the subsequent
    // navigation. `<link rel="prefetch" as="document">` routes into Chromium's
    // stricter document-prefetch cache with extra reuse conditions, and we
    // saw it fail to serve the follow-up click in practice.
    // `credentials: "same-origin"` is the default but we spell it out: auth'd
    // apps need cookies on the prefetch for the HTTP cache entry to match the
    // navigation request.
    function prefetch(url) {
        seen.add(url);
        try {
            fetch(url, {
                credentials: "same-origin",
                redirect: "manual",   // don't follow 302s; they'd prefetch login pages
                headers: { "Purpose": "prefetch", "X-Soli-Prefetch": "1" }
            }).catch(function () {});
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
