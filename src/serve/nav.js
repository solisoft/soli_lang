// Soli instant navigation ("nav"). Embedded verbatim in the soli binary;
// served as an external script at /__soli/nav.js so strict CSP (no
// unsafe-inline) works. Auto-injected into every HTML response unless
// SOLI_NAV=off — when nav is on it replaces the hover-prefetch script and
// takes over prefetching with an in-memory cache (a fetch() can't consume
// `<link rel="prefetch" as="document">` entries: cache partitioning keys
// navigations and cors fetches separately, see prefetch.js).
//
// What it does: intercepts same-origin left-clicks on plain GET links,
// fetches the page, swaps <body> in place (merging <title>/stylesheets/meta
// from the new <head>), and manages history with pushState/popstate — so
// navigation keeps CSS/JS warm and feels instant while the app stays plain
// server-rendered HTML.
//
// Opt-outs:
//   - per link / subtree:  <a data-no-nav> or any ancestor with that attribute
//   - per page:            <meta name="soli-nav" content="off">
//   - globally:            SOLI_NAV=off (server stops injecting this script)
//
// Events (on document):
//   - soli:visit          cancelable; fired before a visit starts
//   - soli:before-render  cancelable; fired with {newDocument} before swap
//   - soli:load           fired after each swap — the DOMContentLoaded
//                         replacement for re-initializing widgets
(function () {
    if (window.__soliNavInstalled) return;
    window.__soliNavInstalled = true;

    function metaOff(doc) {
        var m = doc.querySelector('meta[name="soli-nav"]');
        return !!(m && /^(off|false|0|no)$/i.test(m.getAttribute("content") || ""));
    }
    if (metaOff(document)) return;

    function importMapText(doc) {
        var m = doc.querySelector('script[type="importmap"]');
        return m ? (m.textContent || "").replace(/\s+/g, " ").trim() : "";
    }
    // True when `doc` carries an import map the current page doesn't already
    // have registered — the one case a body swap can't honor (see render()).
    function newImportMap(doc) {
        var incoming = importMapText(doc);
        return !!incoming && incoming !== importMapText(document);
    }

    // Config attributes stamped on our own <script> tag by the server.
    var me = document.querySelector('script[src^="/__soli/nav.js"]');
    var prefetchOn = !(me && me.getAttribute("data-prefetch") === "off");
    var ttlMs = ((me && parseInt(me.getAttribute("data-prefetch-ttl"), 10)) || 30) * 1000;

    // We restore scroll ourselves on popstate (the browser's automatic
    // restoration fires before we've swapped the old body back in).
    if ("scrollRestoration" in history) history.scrollRestoration = "manual";
    history.replaceState({ soli: true }, "", location.href);
    var lastRenderedUrl = location.href;

    var conn = navigator.connection;
    var slowNet = !!(conn && (conn.saveData || /2g/.test(conn.effectiveType || "")));

    // --------------------------------------------- DOMContentLoaded replay
    // Page init code routinely waits on DOMContentLoaded (or window load).
    // Those events fire once per document — never again after a body swap —
    // so a re-executed inline script that registers such a listener would
    // wait forever (the slideshow that works on first load and vanishes
    // after navigating back). Give late registrations jQuery-ready
    // semantics instead: once the event has fired, registering a listener
    // for it invokes the listener asynchronously. Listeners registered
    // before the event fires behave natively. Re-dispatching a synthetic
    // DOMContentLoaded globally is NOT an option — it would re-trigger the
    // bootstrap listeners of already-loaded libraries (Alpine's CDN build
    // starts on DOMContentLoaded; firing it again double-starts every
    // component). Per-listener replay only touches new registrations.
    // alpine:init gets the same treatment: it fires once per tab when Alpine
    // starts, so a page-specific bundle first executed by a swap — the
    // canonical `document.addEventListener("alpine:init", () =>
    // Alpine.data(...))` pattern — would register its components into the
    // void and every x-data binding would throw ReferenceError.
    var dclFired = document.readyState === "complete";
    var loadFired = dclFired;
    if (!dclFired) {
        document.addEventListener("DOMContentLoaded", function () { dclFired = true; });
        window.addEventListener("load", function () { loadFired = true; });
    }
    // A flag listener alone can't observe Alpine starting: deferred scripts
    // run with readyState already "interactive", so Alpine's CDN build takes
    // its already-parsed branch and starts synchronously during its own
    // evaluation — i.e. BEFORE this (later) deferred script registers
    // anything. `window.Alpine` existing is the reliable "alpine:init has
    // fired (or registration is safe now)" signal: Alpine.data() before
    // start is the documented pattern, so an early replay is harmless.
    var alpineInitFired = false;
    document.addEventListener("alpine:init", function () { alpineInitFired = true; });
    function alpineStarted() { return alpineInitFired || !!window.Alpine; }

    function invokeReplayed(target, type, listener) {
        setTimeout(function () {
            try {
                if (typeof listener === "function") listener.call(target, new Event(type));
                else if (listener.handleEvent) listener.handleEvent(new Event(type));
            } catch (e) {
                if (window.console && console.error) console.error(e);
            }
        }, 0);
    }

    function patchAddEventListener(target, replayable) {
        var orig = target.addEventListener.bind(target);
        target.addEventListener = function (type, listener, options) {
            var hasFired = replayable[type];
            if (hasFired && hasFired() && listener) {
                invokeReplayed(target, type, listener);
                return;
            }
            return orig(type, listener, options);
        };
    }
    patchAddEventListener(document, {
        "DOMContentLoaded": function () { return dclFired; },
        "alpine:init": alpineStarted,
        "alpine:initialized": alpineStarted
    });
    patchAddEventListener(window, {
        // DOMContentLoaded bubbles to window; some code listens there.
        "DOMContentLoaded": function () { return dclFired; },
        "load": function () { return loadFired; }
    });

    // External scripts that already executed in this document. Re-evaluating
    // a library on swap is the sharpest edge here: a second alpine.min.js
    // double-starts every component. Externals run once per src; inline
    // scripts re-run on every swap (that's what page-specific init wants).
    var executedSrcs = new Set();
    function absUrl(src) {
        try { return new URL(src, location.href).href; } catch (e) { return src; }
    }
    document.querySelectorAll("script[src]").forEach(function (s) {
        executedSrcs.add(absUrl(s.getAttribute("src")));
    });

    // ---------------------------------------------------------------- fetch

    function fetchPage(url, isPrefetch, signal) {
        // X-Soli-Nav is informational only — the server must never vary
        // response bytes on it, or the shared ETag/304 reuse breaks.
        var headers = {
            "Accept": "text/html, application/xhtml+xml",
            "X-Soli-Nav": "1"
        };
        // The server already recognizes `Purpose: prefetch` and answers with
        // `private, max-age=TTL`, so the entry revalidates cheaply later.
        if (isPrefetch) headers["Purpose"] = "prefetch";
        return fetch(url, {
            headers: headers,
            credentials: "same-origin",
            redirect: "follow",
            signal: signal
        }).then(function (r) {
            var ct = r.headers.get("content-type") || "";
            return r.text().then(function (html) {
                return {
                    html: html,
                    url: r.url,
                    status: r.status,
                    isHtml: ct.indexOf("text/html") !== -1
                };
            });
        });
    }

    // -------------------------------------------------------- hover prefetch

    var cache = new Map(); // url -> { promise, time }

    function consumeCache(url) {
        var entry = cache.get(url);
        cache.delete(url);
        if (entry && Date.now() - entry.time < ttlMs) return entry.promise;
        return null;
    }

    function shouldPrefetch(a) {
        if (!prefetchOn || slowNet) return false;
        if (!shouldIntercept(a)) return false;
        // Self-link: no navigation to accelerate (hash ignored on purpose).
        if (a.pathname === location.pathname && a.search === location.search) return false;
        if (a.hasAttribute("data-no-prefetch")) return false;
        if (a.closest("[data-no-prefetch]")) return false;
        if (cache.has(a.href)) return false;
        return true;
    }

    function prefetch(url) {
        cache.set(url, {
            promise: fetchPage(url, true, null),
            time: Date.now()
        });
    }

    var hoverTimer = null;
    var HOVER_DELAY = 65; // ms — avoid prefetching fly-over hovers

    document.addEventListener("mouseover", function (e) {
        var a = e.target.closest && e.target.closest("a[href]");
        if (!a || !shouldPrefetch(a)) return;
        if (hoverTimer) clearTimeout(hoverTimer);
        hoverTimer = setTimeout(function () { prefetch(a.href); }, HOVER_DELAY);
    }, { passive: true });

    document.addEventListener("mouseout", function () {
        if (hoverTimer) { clearTimeout(hoverTimer); hoverTimer = null; }
    }, { passive: true });

    // Touch: no hover, but touchstart is a strong intent signal.
    document.addEventListener("touchstart", function (e) {
        var a = e.target.closest && e.target.closest("a[href]");
        if (a && shouldPrefetch(a)) prefetch(a.href);
    }, { passive: true });

    // ----------------------------------------------------- click interception

    function shouldIntercept(a) {
        if (!(a instanceof HTMLAnchorElement)) return false; // excludes SVG <a>
        if (!a.href) return false;
        if (a.origin !== location.origin) return false;
        if (a.protocol !== "http:" && a.protocol !== "https:") return false;
        if (a.target && a.target !== "_self") return false;
        if (a.hasAttribute("download")) return false;
        var method = (a.getAttribute("data-method") || "get").toLowerCase();
        if (method !== "get") return false;
        if (a.hasAttribute("data-no-nav")) return false;
        if (a.closest("[data-no-nav]")) return false;
        // htmx manages its own requests — never fight it.
        for (var i = 0; i < a.attributes.length; i++) {
            var n = a.attributes[i].name;
            if (n.indexOf("hx-") === 0 || n.indexOf("data-hx-") === 0) return false;
        }
        if (a.closest("[hx-boost],[data-hx-boost]")) return false;
        return true;
    }

    // Bubble phase: element-level handlers (htmx, Alpine @click) run first
    // and can preventDefault() to keep us out.
    document.addEventListener("click", function (e) {
        if (e.defaultPrevented) return;
        if (e.button !== 0) return;
        if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
        var a = e.target.closest && e.target.closest("a[href]");
        if (!a || !shouldIntercept(a)) return;
        var samePage = a.pathname === location.pathname && a.search === location.search;
        // Hash-only same-page link: let the browser do its native anchor scroll.
        if (samePage && a.hash) return;
        e.preventDefault();
        visit(a.href, { action: samePage ? "replace" : "push", hash: a.hash });
    });

    // ------------------------------------------------------------------ visit

    var inflight = null;

    function visit(url, opts) {
        var ev = new CustomEvent("soli:visit", { cancelable: true, detail: { url: url } });
        if (!document.dispatchEvent(ev)) { location.assign(url); return; }

        if (inflight) inflight.abort();
        var ac = inflight = new AbortController();

        var page = consumeCache(url) || fetchPage(url, false, ac.signal);
        page.then(function (p) {
            if (ac !== inflight) return; // superseded by a newer click
            inflight = null;
            // Downloads, JSON endpoints, …: hand back to the browser.
            if (!p.isHtml) { location.assign(url); return; }
            // A redirect chain that left our origin can't be swapped in.
            if (new URL(p.url).origin !== location.origin) { location.assign(p.url); return; }
            render(p, opts);
        }).catch(function (err) {
            if (err && err.name === "AbortError") return;
            inflight = null;
            // Graceful degradation: do the navigation for real. On popstate
            // the URL bar already shows the destination, so reload instead.
            if (opts.pop) location.reload();
            else location.assign(url);
        });
    }

    // ----------------------------------------------------------------- render

    function render(page, opts) {
        var doc = new DOMParser().parseFromString(page.html, "text/html");

        // The target page refuses swapping — honor it with a real navigation.
        if (metaOff(doc)) { location.assign(page.url); return; }
        // Teleported Alpine trees (clones at the end of <body>) can't survive
        // a body swap — same reasoning as the live-reload morpher.
        if (document.querySelector("template[x-teleport]")) { location.assign(page.url); return; }
        // Import maps must exist before any module script loads, and can't be
        // reliably registered into a live document after the fact — so a page
        // that introduces an <script type="importmap"> the current page lacks
        // can't be body-swapped: its module scripts' bare imports (e.g.
        // `import "three"`) would fail to resolve and the page renders blank.
        // Hand off to a real navigation so the browser parses the map natively.
        if (newImportMap(doc)) { location.assign(page.url); return; }

        var ev = new CustomEvent("soli:before-render", {
            cancelable: true,
            detail: { newDocument: doc }
        });
        if (!document.dispatchEvent(ev)) { location.assign(page.url); return; }

        addNewStylesheets(doc).then(function () {
            // fetch() resolves redirects, so page.url is the final URL — but it
            // drops the fragment, so re-append the original link's hash.
            var finalUrl = page.url + (opts.hash || "");
            if (opts.action === "push") {
                // Stamp the departing entry with its scroll offset so a later
                // back-navigation can restore it.
                history.replaceState(
                    { soli: true, scroll: [window.scrollX, window.scrollY] },
                    "", location.href
                );
                history.pushState({ soli: true }, "", finalUrl);
            } else if (opts.action === "replace") {
                history.replaceState({ soli: true }, "", finalUrl);
            }
            // action "none": popstate already moved the history entry.
            lastRenderedUrl = location.href;

            var doSwap = function () { swap(doc); };
            // Scripts run AFTER the body is attached, sequentially, and the
            // Alpine/htmx re-init waits for them (see executeScripts). The
            // view transition wraps only the DOM swap — awaiting script
            // downloads inside the transition callback would freeze rendering
            // on the old-page snapshot until a CDN responds.
            var finish = function () {
                scrollAndFocus(opts);
                executeScripts(document.body).then(function () {
                    // One macrotask later: replayed listeners (DOMContentLoaded,
                    // alpine:init) were scheduled via setTimeout during script
                    // execution and must run before Alpine.initTree sees the
                    // new body.
                    setTimeout(initNewBody, 0);
                });
            };
            // View transitions are opt-in via the same meta tag Turbo uses.
            var vt = document.querySelector('meta[name="view-transition"][content="same-origin"]');
            if (vt && document.startViewTransition) {
                var transition = document.startViewTransition(doSwap);
                var settled = transition && transition.finished ? transition.finished : Promise.resolve();
                settled.then(finish, finish);
            } else {
                doSwap();
                finish();
            }
        });
    }

    // Append stylesheets the new page needs but the current one lacks, and
    // wait for them (capped) so the swapped body never renders unstyled.
    function addNewStylesheets(doc) {
        var current = new Set();
        document.querySelectorAll('link[rel="stylesheet"]').forEach(function (l) {
            var href = l.getAttribute("href");
            if (href) current.add(href.split("?")[0]);
        });
        var waits = [];
        doc.querySelectorAll('link[rel="stylesheet"]').forEach(function (l) {
            var href = l.getAttribute("href");
            if (!href || current.has(href.split("?")[0])) return;
            var link = document.createElement("link");
            link.rel = "stylesheet";
            link.href = href;
            waits.push(new Promise(function (resolve) {
                link.onload = link.onerror = resolve;
                setTimeout(resolve, 500); // FOUC cap — don't block forever
            }));
            document.head.appendChild(link);
        });
        return Promise.all(waits);
    }

    function swap(doc) {
        // ---- head merge ----
        document.title = doc.title;

        var newHrefs = new Set();
        doc.querySelectorAll('link[rel="stylesheet"]').forEach(function (l) {
            var href = l.getAttribute("href");
            if (href) newHrefs.add(href.split("?")[0]);
        });
        document.querySelectorAll('link[rel="stylesheet"]').forEach(function (l) {
            var href = l.getAttribute("href");
            if (href && !newHrefs.has(href.split("?")[0])) l.remove();
        });

        // Inline <style>: replace wholesale, except the live-reload marker
        // styles the dev tooling owns.
        document.querySelectorAll("head style").forEach(function (s) {
            if (!s.textContent.includes("__livereload")) s.remove();
        });
        doc.querySelectorAll("head style").forEach(function (s) {
            if (!s.textContent.includes("__livereload")) {
                document.head.appendChild(document.adoptNode(s));
            }
        });

        // meta[name]/meta[property] (description, og:*, csrf, …): adopt the
        // new page's. Skip viewport/charset and our own control metas.
        var SKIP_META = /^(viewport|soli-nav|view-transition)$/i;
        document.querySelectorAll("head meta[name], head meta[property]").forEach(function (m) {
            var key = m.getAttribute("name") || m.getAttribute("property");
            if (!SKIP_META.test(key)) m.remove();
        });
        doc.querySelectorAll("head meta[name], head meta[property]").forEach(function (m) {
            var key = m.getAttribute("name") || m.getAttribute("property");
            if (!SKIP_META.test(key)) document.head.appendChild(document.adoptNode(m));
        });

        // ---- body swap ----
        if (window.Alpine && window.Alpine.destroyTree) {
            try { window.Alpine.destroyTree(document.body); } catch (e) { /* older 3.x */ }
        }
        // DOMParser-created <script> elements carry the "already started"
        // flag, so attaching them executes nothing — executeScripts revives
        // them afterwards, one by one, in document order.
        var newBody = document.adoptNode(doc.body);
        // Alpine's global MutationObserver auto-initializes any subtree
        // added to the document — i.e. on this replaceChild, before the new
        // page's scripts have run, so x-data components those scripts
        // register would all evaluate to ReferenceErrors. x-ignore makes
        // Alpine skip the tree; initNewBody lifts it and initializes
        // manually once the script chain has settled.
        if (window.Alpine) newBody.setAttribute("x-ignore", "");
        document.documentElement.replaceChild(newBody, document.body);
    }

    // Re-execute the new body's scripts SEQUENTIALLY in document order,
    // awaiting each external before moving on — the same ordering the parser
    // guarantees on a full load. Naively activating them all at once inverts
    // it: a fresh inline script executes synchronously on insertion while a
    // fresh external only executes after download, so
    //   <script src="cdn.tailwindcss.com"></script>
    //   <script>tailwind.config = {...}</script>
    // throws `tailwind is not defined` after a swap. Returns a promise that
    // resolves when every script has run, so Alpine/htmx re-init can't race
    // a page-specific external (e.g. an editor bundle registering
    // Alpine.data components) that hasn't loaded yet.
    function executeScripts(root) {
        var queue = [];
        root.querySelectorAll("script").forEach(function (old) {
            var type = old.getAttribute("type");
            if (type && !/javascript|module/.test(type)) return; // data blocks etc.
            var src = old.getAttribute("src");
            if (src) {
                var abs = absUrl(src);
                // Ourselves / prefetch.js: already running in this document.
                if (new URL(abs).pathname.indexOf("/__soli/") === 0) return;
                // Libraries already evaluated (alpine, htmx, …) must not
                // double-start. New srcs run and join the set.
                if (executedSrcs.has(abs)) return;
                executedSrcs.add(abs);
            } else if (old.textContent.includes("__livereload")) {
                // The dev live-reload IIFE survives the swap (its WS lives in
                // a closure); its window guard would no-op a re-run anyway.
                return;
            }
            queue.push(old);
        });
        return queue.reduce(function (chain, old) {
            return chain.then(function () { return runScript(old); });
        }, Promise.resolve());
    }

    function runScript(old) {
        // A prior script may have removed this node from the DOM.
        if (!old.parentNode) return Promise.resolve();
        var fresh = document.createElement("script");
        for (var i = 0; i < old.attributes.length; i++) {
            fresh.setAttribute(old.attributes[i].name, old.attributes[i].value);
        }
        fresh.textContent = old.textContent;
        if (!old.getAttribute("src")) {
            // Inline: executes synchronously on insertion. An exception in it
            // surfaces on window.onerror, not here — the chain continues.
            old.parentNode.replaceChild(fresh, old);
            return Promise.resolve();
        }
        return new Promise(function (resolve) {
            var done = false;
            var finish = function () { if (!done) { done = true; resolve(); } };
            fresh.onload = fresh.onerror = finish;
            // Dead-CDN safety: never wedge the visit on one hanging script.
            setTimeout(finish, 10000);
            old.parentNode.replaceChild(fresh, old);
        });
    }

    function scrollAndFocus(opts) {
        if (opts.scroll) {
            window.scrollTo(opts.scroll[0], opts.scroll[1]);
        } else if (location.hash) {
            var target = document.getElementById(location.hash.slice(1));
            if (target) target.scrollIntoView();
            else window.scrollTo(0, 0);
        } else {
            window.scrollTo(0, 0);
        }

        var af = document.querySelector("[autofocus]");
        if (af) { try { af.focus(); } catch (e) { /* ignore */ } }
    }

    // Runs only after executeScripts settles, so page-level Alpine.data()
    // registrations and freshly-loaded externals exist before the tree
    // initializes — initializing earlier evaluates x-data scopes that aren't
    // registered yet and every binding throws ReferenceError.
    function initNewBody() {
        if (window.Alpine) {
            document.body.removeAttribute("x-ignore");
            try { delete document.body._x_ignore; } catch (e) { /* ignore */ }
            if (window.Alpine.initTree) window.Alpine.initTree(document.body);
        }
        if (window.htmx && window.htmx.process) {
            window.htmx.process(document.body);
        }

        document.dispatchEvent(new CustomEvent("soli:load", {
            detail: { url: location.href }
        }));
    }

    // --------------------------------------------------------------- popstate

    window.addEventListener("popstate", function (e) {
        var now = new URL(location.href);
        var last = new URL(lastRenderedUrl);
        // Hash-only traversal on the same page: just scroll.
        if (now.pathname === last.pathname && now.search === last.search) {
            lastRenderedUrl = location.href;
            if (now.hash) {
                var target = document.getElementById(now.hash.slice(1));
                if (target) target.scrollIntoView();
            } else {
                window.scrollTo(0, 0);
            }
            return;
        }
        // Refetch — cheap: the server's `private, no-cache` + weak ETag turn
        // this into a conditional GET answered 304 from the HTTP cache.
        visit(location.href, {
            action: "none",
            pop: true,
            scroll: e.state && e.state.soli && e.state.scroll
        });
    });
})();
