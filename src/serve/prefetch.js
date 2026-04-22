// Soli hover-prefetch. Embedded verbatim in the soli binary; served as an
// external script at /__soli/prefetch.js so strict CSP (no unsafe-inline) works.
// Auto-injected into every HTML response unless SOLI_PREFETCH=off.
// Per-link opt-out: <a data-no-prefetch> or any ancestor with that attribute.
(function () {
    if (window.__soliPrefetchInstalled) return;
    window.__soliPrefetchInstalled = true;

    var DELAY = 65;       // ms — avoid prefetching fly-over hovers
    var CLEANUP = 2000;   // ms — remove <link> from head after this window
    var seen = new Set();

    var conn = navigator.connection;
    if (conn && (conn.saveData || /2g/.test(conn.effectiveType || ""))) return;

    function shouldPrefetch(a) {
        if (!a.href) return false;
        if (a.origin !== location.origin) return false;
        if (a.hash && a.pathname === location.pathname) return false; // in-page
        if (a.protocol !== "http:" && a.protocol !== "https:") return false;
        if (a.hasAttribute("data-no-prefetch")) return false;
        if (a.closest("[data-no-prefetch]")) return false;
        var method = (a.getAttribute("data-method") || "get").toLowerCase();
        if (method !== "get") return false;
        if (seen.has(a.href)) return false;
        return true;
    }

    function prefetch(url) {
        seen.add(url);
        var link = document.createElement("link");
        link.rel = "prefetch";
        link.href = url;
        link.as = "document";
        document.head.appendChild(link);
        setTimeout(function () { link.remove(); }, CLEANUP);
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
