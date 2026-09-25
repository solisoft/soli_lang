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
// Morph (opt-in): <meta name="soli-nav" content="morph"> on the incoming page,
// or SOLI_NAV=morph for every page (a page can still say content="swap"),
// patches the current <body> into the new one instead of replacing it. Nodes
// that did not change stay the same DOM nodes — a layout's header and
// sidebar keep their scroll position, open <details>, typed input and running
// transitions. Elements are paired by id first, then by tag in order. Alpine
// components (x-data) are replaced whole rather than patched, so their state
// resets as with a swap; pages with x-teleport fall back to a swap.
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
    var morphByDefault = !!(me && me.getAttribute("data-mode") === "morph");

    // Morph or swap for this incoming page. The page's own meta wins over the
    // server-wide default; Alpine teleports on either side force a swap (see
    // render()).
    function wantsMorph(doc) {
        var m = doc.querySelector('meta[name="soli-nav"]');
        var mode = m ? (m.getAttribute("content") || "").toLowerCase() : "";
        var morph = mode === "morph" || (morphByDefault && mode !== "swap");
        return morph && !hasTeleport(document) && !hasTeleport(doc);
    }
    function hasTeleport(doc) {
        return !!doc.querySelector("template[x-teleport]");
    }

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
        // LiveView owns these: in-socket patch / component swap / redirect.
        // Instant-nav binds on DOMContentLoaded; LiveView bindEvents runs
        // later (after the first websocket render), so our bubble listener
        // would otherwise steal the click and fetch a new page.
        if (a.hasAttribute("soli-patch") || a.hasAttribute("data-soli-patch") ||
            a.hasAttribute("soli-live") || a.hasAttribute("data-soli-live") ||
            a.hasAttribute("soli-href") || a.hasAttribute("data-soli-href")) {
            return false;
        }
        // htmx manages its own requests — never fight it.
        for (var i = 0; i < a.attributes.length; i++) {
            var n = a.attributes[i].name;
            if (n.indexOf("hx-") === 0 || n.indexOf("data-hx-") === 0) return false;
        }
        if (a.closest("[hx-boost],[data-hx-boost]")) return false;
        return true;
    }

    // `data-confirm`: ask before a destructive submit.
    //
    // This replaces the inline `onclick="return confirm('...')"` the form
    // builder used to emit. That string was JavaScript-escaped, not
    // attribute-escaped, so a confirm message built from record data could
    // close the attribute and open a live event handler — a stored XSS in the
    // most ordinary "Delete <title>?" button. A data attribute is escaped once,
    // for one context, and carries no code.
    //
    // THE APPLICATION ANSWERS FIRST. `soli:confirm` is dispatched on the
    // element and can be cancelled: a page that ships its own dialog calls
    // `preventDefault()` on it and takes over, and the native box never opens.
    // Without a listener the event goes through unchanged, so a page that has
    // no dialog of its own keeps the one it always had.
    //
    // This is not a nicety. `window.confirm` blocks the renderer until someone
    // answers, and nothing answers in a driven session: a browser spec that
    // clicks such a button gets a frozen page, not a failure. An application
    // that already had `data-confirm` — the attribute is not reserved — found
    // its own dialog shadowed by a native box it never asked for, and its
    // whole browser suite stopped returning.
    document.addEventListener("click", function (e) {
        if (e.defaultPrevented) return;
        var el = e.target.closest && e.target.closest("[data-confirm]");
        if (!el) return;
        var message = el.getAttribute("data-confirm");
        if (!message) return;
        var ask = new CustomEvent("soli:confirm", {
            bubbles: true,
            cancelable: true,
            detail: { message: message, element: el }
        });
        // `dispatchEvent` returns false when a listener cancelled it.
        if (!el.dispatchEvent(ask)) return;
        if (!window.confirm(message)) {
            e.preventDefault();
            e.stopPropagation();
        }
    }, true);

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
        // NOTE: unlike the live-reload morpher, instant-nav does NOT bail on
        // Alpine x-teleport. A morph keeps the old <body> and reconciles it, so
        // teleported clones and their <template> sources desync. A swap instead
        // calls Alpine.destroyTree(document.body) (see swap()) — which runs each
        // teleport's registered cleanup and removes its clone — then replaces
        // the whole body and re-runs initTree, re-teleporting fresh. So teleport
        // pages swap cleanly; bailing here forced a full reload (and tore down
        // any data-soli-permanent widget) on every card/modal page for nothing.
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

            // A morph reports which scripts and Alpine subtrees it brought
            // in; a swap brings in the whole body.
            var morphed = null;
            var doSwap = wantsMorph(doc)
                ? function () { morphed = morph(doc); }
                : function () { swap(doc); };
            // Scripts run AFTER the body is attached, sequentially, and the
            // Alpine/htmx re-init waits for them (see executeScripts). The
            // view transition wraps only the DOM swap — awaiting script
            // downloads inside the transition callback would freeze rendering
            // on the old-page snapshot until a CDN responds.
            var finish = function () {
                scrollAndFocus(opts);
                var scripts = morphed
                    ? morphed.scripts
                    : Array.prototype.slice.call(document.body.querySelectorAll("script"));
                executeScripts(scripts).then(function () {
                    // One macrotask later: replayed listeners (DOMContentLoaded,
                    // alpine:init) were scheduled via setTimeout during script
                    // execution and must run before Alpine.initTree sees the
                    // new body.
                    setTimeout(function () { initNewBody(morphed); }, 0);
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
        mergeHead(doc);
        swapBody(doc);
    }

    function mergeHead(doc) {
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
    }

    function swapBody(doc) {

        // ---- permanent elements ----
        // Elements tagged [data-soli-permanent] (with an id) are lifted out of
        // the outgoing body and grafted over the matching placeholder in the
        // incoming one, untouched — so a live widget (a map, a media/video
        // player, an editor) keeps running across navigation instead of being
        // torn down and rebuilt. Persists only between pages that BOTH declare
        // the element (Turbo's data-turbo-permanent semantics): if the new page
        // has no matching placeholder, the node leaves with the old body.
        // Collected BEFORE Alpine.destroyTree below so its subtree survives.
        var permanents = [];
        document.querySelectorAll("[data-soli-permanent][id]").forEach(function (el) {
            permanents.push(el);
        });

        // ---- body swap ----
        if (window.Alpine && window.Alpine.destroyTree) {
            try { window.Alpine.destroyTree(document.body); } catch (e) { /* older 3.x */ }
        }
        // DOMParser-created <script> elements carry the "already started"
        // flag, so attaching them executes nothing — executeScripts revives
        // them afterwards, one by one, in document order.
        var newBody = document.adoptNode(doc.body);
        // Graft each surviving permanent node over its placeholder in the new
        // body. Moving a node still parented in the live document.body into
        // newBody detaches it cleanly, so it's gone before the replaceChild
        // discards the old body. No matching placeholder → node is dropped.
        permanents.forEach(function (node) {
            var placeholder = newBody.querySelector(
                "#" + (window.CSS && CSS.escape ? CSS.escape(node.id) : node.id) + "[data-soli-permanent]"
            );
            if (placeholder) placeholder.parentNode.replaceChild(node, placeholder);
        });
        // Alpine's global MutationObserver auto-initializes any subtree
        // added to the document — i.e. on this replaceChild, before the new
        // page's scripts have run, so x-data components those scripts
        // register would all evaluate to ReferenceErrors. x-ignore makes
        // Alpine skip the tree; initNewBody lifts it and initializes
        // manually once the script chain has settled.
        if (window.Alpine) newBody.setAttribute("x-ignore", "");
        document.documentElement.replaceChild(newBody, document.body);
    }

    // ------------------------------------------------------------------ morph

    // Patch the live body into the new one. Returns the scripts it inserted
    // (to run, in document order) and the element subtrees it inserted (for
    // Alpine to initialize once those scripts have run).
    function morph(doc) {
        mergeHead(doc);
        var ctx = { scripts: [], alpineRoots: [] };
        morphAttributes(document.body, doc.body);
        morphChildren(document.body, doc.body, ctx);
        return ctx;
    }

    // Two nodes can be patched one into the other: same kind, same tag, and
    // the same id — an element with an id only ever pairs with its namesake.
    function compatible(a, b) {
        if (a.nodeType !== b.nodeType) return false;
        if (a.nodeType !== 1) return true;
        return a.tagName === b.tagName && (a.id || "") === (b.id || "");
    }

    function morphChildren(oldParent, newParent, ctx) {
        var byId = Object.create(null);
        for (var c = oldParent.firstChild; c; c = c.nextSibling) {
            if (c.nodeType === 1 && c.id) byId[c.id] = c;
        }
        var cursor = oldParent.firstChild;
        // Snapshot: inserting a node adopts it out of newParent.
        var incoming = Array.prototype.slice.call(newParent.childNodes);
        incoming.forEach(function (fresh) {
            var match = null;
            if (fresh.nodeType === 1 && fresh.id) {
                var named = byId[fresh.id];
                if (named && named.tagName === fresh.tagName) match = named;
            } else if (cursor && compatible(cursor, fresh)) {
                match = cursor;
            } else if (cursor) {
                // Skip past a few nodes the new page no longer has (a flash
                // message, a banner) rather than rebuilding everything after.
                var probe = cursor.nextSibling;
                for (var n = 0; probe && n < 3 && !match; n++, probe = probe.nextSibling) {
                    if (compatible(probe, fresh)) match = probe;
                }
            }
            if (match) {
                if (match.id) delete byId[match.id];
                if (match === cursor) cursor = cursor.nextSibling;
                else oldParent.insertBefore(match, cursor);
                morphNode(match, fresh, ctx);
            } else {
                oldParent.insertBefore(adopt(fresh, ctx), cursor);
            }
        });
        while (cursor) {
            var next = cursor.nextSibling;
            discard(cursor);
            cursor = next;
        }
    }

    function morphNode(old, fresh, ctx) {
        if (old.nodeType !== 1) {
            if (old.nodeValue !== fresh.nodeValue) old.nodeValue = fresh.nodeValue;
            return;
        }
        // Carried over untouched, exactly as a swap grafts it.
        if (old.id && old.hasAttribute("data-soli-permanent")) return;
        // A script that did not change has run and must not run again; one
        // that changed is new code and is replaced, to be run.
        if (old.tagName === "SCRIPT") {
            if (!sameScript(old, fresh)) replaceNode(old, fresh, ctx);
            return;
        }
        // Alpine rewrites what it renders, so the live DOM of a component no
        // longer matches any server HTML to diff against: replace it whole.
        if (window.Alpine && (old.hasAttribute("x-data") || fresh.hasAttribute("x-data"))) {
            replaceNode(old, fresh, ctx);
            return;
        }
        if (old.isEqualNode(fresh)) return;
        var serverValue = old.getAttribute("value");
        morphAttributes(old, fresh);
        if (old.tagName === "INPUT") {
            // Keep what the user typed, unless the server changed the value.
            if (fresh.getAttribute("value") !== serverValue) old.value = fresh.value;
            if (old.checked !== fresh.hasAttribute("checked") &&
                old.defaultChecked === fresh.hasAttribute("checked")) {
                old.checked = fresh.hasAttribute("checked");
            }
            return;
        }
        if (old.tagName === "TEXTAREA") {
            if (old.defaultValue !== fresh.defaultValue) {
                old.defaultValue = fresh.defaultValue;
                old.value = fresh.defaultValue;
            }
            return;
        }
        if (old.tagName === "TEMPLATE") {
            old.innerHTML = fresh.innerHTML;
            return;
        }
        morphChildren(old, fresh, ctx);
    }

    function morphAttributes(old, fresh) {
        // `open` on <details>/<dialog> is the reader's, like a typed value:
        // opening one writes the attribute, and the server's HTML never has it.
        var keepOpen = old.tagName === "DETAILS" || old.tagName === "DIALOG";
        var i, attr;
        for (i = old.attributes.length - 1; i >= 0; i--) {
            attr = old.attributes[i];
            if (keepOpen && attr.name === "open") continue;
            if (!fresh.hasAttribute(attr.name)) old.removeAttribute(attr.name);
        }
        for (i = 0; i < fresh.attributes.length; i++) {
            attr = fresh.attributes[i];
            if (keepOpen && attr.name === "open") continue;
            if (old.getAttribute(attr.name) !== attr.value) old.setAttribute(attr.name, attr.value);
        }
    }

    function sameScript(a, b) {
        return a.getAttribute("src") === b.getAttribute("src") &&
            a.getAttribute("type") === b.getAttribute("type") &&
            a.textContent === b.textContent;
    }

    function replaceNode(old, fresh, ctx) {
        var node = adopt(fresh, ctx);
        destroyAlpine(old);
        old.parentNode.replaceChild(node, old);
    }

    // Bring a node from the parsed page into this document. Its scripts are
    // queued to run; with Alpine present it is x-ignored until those scripts
    // have registered their components (see swapBody for why).
    function adopt(fresh, ctx) {
        var node = document.adoptNode(fresh);
        if (node.nodeType !== 1) return node;
        if (node.tagName === "SCRIPT") ctx.scripts.push(node);
        else Array.prototype.push.apply(ctx.scripts, node.querySelectorAll("script"));
        if (window.Alpine) {
            node.setAttribute("x-ignore", "");
            ctx.alpineRoots.push(node);
        }
        return node;
    }

    function discard(node) {
        destroyAlpine(node);
        node.parentNode.removeChild(node);
    }

    function destroyAlpine(node) {
        if (node.nodeType === 1 && window.Alpine && window.Alpine.destroyTree) {
            try { window.Alpine.destroyTree(node); } catch (e) { /* older 3.x */ }
        }
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
    function executeScripts(scripts) {
        var queue = [];
        scripts.forEach(function (old) {
            // Scripts inside a permanent element are carried over live and have
            // already executed — re-running them would double-fire.
            if (old.closest("[data-soli-permanent]")) return;
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
    function initNewBody(morphed) {
        if (window.Alpine) {
            // A swap initializes the whole new body; a morph only the subtrees
            // it inserted — the kept ones are live and already initialized.
            var roots = morphed ? morphed.alpineRoots : [document.body];
            roots.forEach(function (root) {
                if (!root.isConnected) return;
                root.removeAttribute("x-ignore");
                try { delete root._x_ignore; } catch (e) { /* ignore */ }
                if (window.Alpine.initTree) window.Alpine.initTree(root);
            });
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
