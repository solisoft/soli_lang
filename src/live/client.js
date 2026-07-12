/**
 * SoliLang LiveView Client
 *
 * Served by the soli binary at /live/client.js so the client always matches
 * the server's patch protocol (shadow line-splices + DOM morphing).
 * ~30 KB raw, ~7 KB gzipped.
 *
 * The server ships positional line splices against the exact HTML string it
 * last sent. The client applies them to a shadow copy of that string, then
 * morphs the live DOM to match — updating nodes in place instead of
 * replacing them, so focus, caret position, and client-side widget state
 * (Alpine, charts, ...) survive patches.
 */

(function(global) {
    'use strict';

    /**
     * LiveView connection state
     */
    const State = {
        DISCONNECTED: 0,
        CONNECTING: 1,
        CONNECTED: 2,
        RECONNECTING: 3
    };

    // ------------------------------------------------------------------
    // Shadow splice
    // ------------------------------------------------------------------

    /**
     * Apply a positional line splice to the shadow HTML string: replace
     * `del` lines starting at line `at` with the lines in `ins`.
     * Returns the new string, or null when the patch is malformed or out
     * of bounds (caller should resync with the server).
     */
    function spliceLines(shadow, at, del, ins) {
        if (!Number.isInteger(at) || !Number.isInteger(del) || !Array.isArray(ins)) {
            return null;
        }
        const lines = shadow.split('\n');
        if (at < 0 || del < 0 || at + del > lines.length) {
            return null;
        }
        lines.splice(at, del, ...ins);
        return lines.join('\n');
    }

    // ------------------------------------------------------------------
    // DOM morphing
    // ------------------------------------------------------------------

    /**
     * Identity key for keyed reconciliation: soli-key attribute, else id.
     */
    function nodeKey(el) {
        return el.getAttribute('soli-key') ||
               el.getAttribute('data-soli-key') ||
               el.id ||
               null;
    }

    /**
     * soli-ignore marks a subtree as client-owned: the morph keeps the
     * element's own attributes in sync but never touches its children.
     */
    function isIgnored(el) {
        return el.hasAttribute('soli-ignore') || el.hasAttribute('data-soli-ignore');
    }

    /**
     * Scripts inside live regions never execute on patch (same contract as
     * the historical innerHTML-based client). Template-parsed scripts WOULD
     * run when inserted into the document, so incoming ones are swapped for
     * inert copies (innerHTML-parsed scripts carry the "already started"
     * flag and stay dormant).
     */
    function inertScript(script) {
        const holder = document.createElement('div');
        holder.innerHTML = script.outerHTML;
        return holder.firstChild;
    }

    /**
     * Prepare a template-parsed node for insertion into the document:
     * neutralize any scripts in the subtree.
     */
    function prepareIncoming(node) {
        if (node.nodeType !== 1) return node;
        if (node.localName === 'script') return inertScript(node);
        const scripts = node.querySelectorAll('script');
        for (const s of scripts) {
            s.parentNode.replaceChild(inertScript(s), s);
        }
        return node;
    }

    /**
     * Reposition a live node, preserving its state where the browser can.
     * Node.moveBefore (newer Chromium) keeps focus, selection, and iframe
     * state across the move; insertBefore is the universal fallback.
     */
    function moveBefore(parent, node, before) {
        if (typeof parent.moveBefore === 'function') {
            try {
                parent.moveBefore(node, before);
                return;
            } catch (_) { /* fall through to insertBefore */ }
        }
        parent.insertBefore(node, before);
    }

    /**
     * Sync attributes from `to` onto the live element `from`.
     * Namespaced attributes (xlink:href, ...) go through the NS API.
     */
    function syncAttributes(from, to) {
        // Remove attributes no longer present (backwards: live NamedNodeMap).
        const fromAttrs = from.attributes;
        for (let i = fromAttrs.length - 1; i >= 0; i--) {
            const attr = fromAttrs[i];
            if (attr.namespaceURI) {
                if (!to.hasAttributeNS(attr.namespaceURI, attr.localName)) {
                    from.removeAttributeNS(attr.namespaceURI, attr.localName);
                }
            } else if (!to.hasAttribute(attr.name)) {
                from.removeAttribute(attr.name);
            }
        }
        // Add and update.
        const toAttrs = to.attributes;
        for (let i = 0; i < toAttrs.length; i++) {
            const attr = toAttrs[i];
            if (attr.namespaceURI) {
                if (from.getAttributeNS(attr.namespaceURI, attr.localName) !== attr.value) {
                    from.setAttributeNS(attr.namespaceURI, attr.name, attr.value);
                }
            } else if (from.getAttribute(attr.name) !== attr.value) {
                from.setAttribute(attr.name, attr.value);
            }
        }
    }

    /**
     * Input state: the value/checked ATTRIBUTES are what the server renders;
     * the PROPERTIES are what the user sees and edits. The user wins: a
     * focused field is never clobbered (so typing that round-trips through
     * soli-change can't lose in-flight keystrokes), and an unfocused field
     * only changes when the server actually changes the rendered attribute.
     */
    function syncInputState(input, to, oldValueAttr, oldCheckedAttr) {
        if (input === document.activeElement) return;

        const newValueAttr = to.getAttribute('value');
        if (newValueAttr !== null && newValueAttr !== oldValueAttr) {
            input.value = newValueAttr;
        }

        const newCheckedAttr = to.hasAttribute('checked');
        if (newCheckedAttr !== oldCheckedAttr) {
            input.checked = newCheckedAttr;
        }
    }

    /**
     * Textarea state: the server renders the value as child text, and the
     * value property detaches from child text once the user types. Same
     * user-wins rule as inputs: never touch the value while focused.
     */
    function syncTextareaState(textarea, to) {
        const newText = to.textContent;
        if (textarea.defaultValue !== newText) {
            textarea.defaultValue = newText;
            if (textarea !== document.activeElement) {
                textarea.value = newText;
            }
        }
    }

    /**
     * Option selectedness follows the same attribute-vs-property rule as
     * inputs; skipped entirely while the parent select is focused (open).
     */
    function syncOptionState(option, to, oldSelectedAttr) {
        const newSelectedAttr = to.hasAttribute('selected');
        if (newSelectedAttr === oldSelectedAttr) return;
        const select = option.closest('select');
        if (select && select === document.activeElement) return;
        option.selected = newSelectedAttr;
    }

    /**
     * Morph a live element to match `to`. Caller guarantees same localName.
     */
    function morphElement(from, to) {
        const tag = from.localName;

        // Scripts: keep identical ones untouched, swap changed ones for an
        // inert copy — never re-execute in place.
        if (tag === 'script') {
            if (from.getAttribute('src') !== to.getAttribute('src') ||
                from.textContent !== to.textContent) {
                from.replaceWith(inertScript(to));
            }
            return;
        }

        // Snapshot server-rendered form attributes BEFORE syncing, so a
        // genuine server change is distinguishable from user edits.
        let oldValueAttr = null;
        let oldCheckedAttr = false;
        let oldSelectedAttr = false;
        if (tag === 'input') {
            oldValueAttr = from.getAttribute('value');
            oldCheckedAttr = from.hasAttribute('checked');
        } else if (tag === 'option') {
            oldSelectedAttr = from.hasAttribute('selected');
        }

        const ignored = isIgnored(from) || isIgnored(to);

        syncAttributes(from, to);

        if (tag === 'input') {
            syncInputState(from, to, oldValueAttr, oldCheckedAttr);
            return;
        }
        if (tag === 'textarea') {
            syncTextareaState(from, to);
            return;
        }
        if (tag === 'iframe') {
            // src changes arrive via syncAttributes (a reload is inherent);
            // never recurse into the frame.
            return;
        }
        if (ignored) {
            // Children are client-owned; attributes stay server-driven.
            return;
        }

        morphChildren(from, to);

        if (tag === 'option') {
            syncOptionState(from, to, oldSelectedAttr);
        }
    }

    /**
     * Morph the children of `fromParent` to match `toParent`'s children.
     * Elements match by key (soli-key attribute, else id), falling back to
     * same-tag-at-same-position; matched nodes are mutated in place so DOM
     * identity — and with it focus and widget state — survives.
     */
    function morphChildren(fromParent, toParent) {
        // Snapshot: adopting nodes out of the template mutates its child list.
        const toKids = Array.from(toParent.childNodes);

        // Index keyed old children so reordered items keep their DOM nodes.
        let oldKeyed = null;
        for (let el = fromParent.firstElementChild; el; el = el.nextElementSibling) {
            const k = nodeKey(el);
            if (k === null || k === '') continue;
            if (oldKeyed === null) oldKeyed = new Map();
            if (oldKeyed.has(k)) {
                console.warn('[LiveView] duplicate soli-key/id in live region:', k);
            } else {
                oldKeyed.set(k, el);
            }
        }

        let cur = fromParent.firstChild;

        for (const toNode of toKids) {
            if (toNode.nodeType === 1) {
                const k = nodeKey(toNode);
                const keyedMatch = (k && oldKeyed) ? oldKeyed.get(k) : undefined;

                if (keyedMatch && keyedMatch.localName === toNode.localName) {
                    oldKeyed.delete(k);
                    if (keyedMatch === cur) {
                        cur = cur.nextSibling;
                    } else {
                        moveBefore(fromParent, keyedMatch, cur);
                    }
                    morphElement(keyedMatch, toNode);
                    continue;
                }

                // Positional match: same tag and same (possibly absent) key.
                if (cur && cur.nodeType === 1 &&
                    cur.localName === toNode.localName &&
                    nodeKey(cur) === k) {
                    const el = cur;
                    cur = cur.nextSibling;
                    morphElement(el, toNode);
                    continue;
                }

                fromParent.insertBefore(prepareIncoming(toNode), cur);
                continue;
            }

            // Text and comment nodes.
            if (cur && cur.nodeType === toNode.nodeType) {
                if (cur.nodeValue !== toNode.nodeValue) {
                    cur.nodeValue = toNode.nodeValue;
                }
                cur = cur.nextSibling;
            } else {
                fromParent.insertBefore(toNode, cur);
            }
        }

        // Whatever old content remains was never matched — remove it.
        while (cur) {
            const next = cur.nextSibling;
            fromParent.removeChild(cur);
            cur = next;
        }
    }

    /**
     * Morph the live region under `root` to match `newHtml`.
     * Both the live DOM and the <template>-parsed target go through the
     * same browser parser, so structure and whitespace align exactly.
     */
    function morph(root, newHtml) {
        const tpl = document.createElement('template');
        tpl.innerHTML = newHtml;
        morphChildren(root, tpl.content);
    }

    /**
     * Main LiveView class
     */
    class SoliLiveView {
        /**
         * Create a new LiveView connection
         * @param {string} socketUrl - WebSocket URL
         * @param {Object} params - Connection parameters
         */
        constructor(socketUrl, params = {}) {
            this.socketUrl = socketUrl;
            this.params = params;
            this.state = State.DISCONNECTED;
            this.socket = null;
            this.reconnectAttempts = 0;
            this.maxReconnectAttempts = 10;
            this.reconnectDelay = 1000;
            this.heartbeatInterval = null;
            this.liveviewId = null;
            this.eventsBound = false;

            // Shadow copy of the exact HTML string last received from the
            // server; splice patches apply to this, then the DOM is morphed
            // to match it.
            this.lastHtml = null;

            // Root element for this LiveView instance
            // Can be set via params.rootElement or params.rootSelector
            this.rootElement = params.rootElement || null;
            this.rootSelector = params.rootSelector || null;

            // Event handlers
            this.eventHandlers = {};

            // Bind methods
            this.connect = this.connect.bind(this);
            this.disconnect = this.disconnect.bind(this);
            this.handleMessage = this.handleMessage.bind(this);
            this.handleOpen = this.handleOpen.bind(this);
            this.handleClose = this.handleClose.bind(this);
            this.handleError = this.handleError.bind(this);
        }

        /**
         * Get the root element for this LiveView instance
         */
        getRoot() {
            if (this.rootElement) {
                return this.rootElement;
            }
            if (this.rootSelector) {
                return document.querySelector(this.rootSelector);
            }
            return document.querySelector('[data-live-root]') || document.body;
        }

        /**
         * Connect to the LiveSocket
         */
        connect() {
            if (this.state !== State.DISCONNECTED && this.state !== State.RECONNECTING) {
                console.warn('Already connected or connecting');
                return;
            }

            this.state = State.CONNECTING;
            this.emit('stateChanged', State.CONNECTING);

            try {
                this.socket = new WebSocket(this.socketUrl);

                this.socket.onopen = this.handleOpen;
                this.socket.onclose = this.handleClose;
                this.socket.onerror = this.handleError;
                this.socket.onmessage = (e) => this.handleMessage(JSON.parse(e.data));
            } catch (e) {
                console.error('Failed to create WebSocket:', e);
                this.scheduleReconnect();
            }
        }

        /**
         * Disconnect from the LiveSocket
         */
        disconnect() {
            if (this.socket) {
                this.socket.close();
                this.socket = null;
            }
            this.state = State.DISCONNECTED;
            this.stopHeartbeat();
            this.emit('stateChanged', State.DISCONNECTED);
        }

        /**
         * Handle WebSocket open
         */
        handleOpen() {
            this.state = State.CONNECTED;
            this.reconnectAttempts = 0;
            this.startHeartbeat();
            this.emit('stateChanged', State.CONNECTED);

            // Send connect message with params
            this.send({
                type: 'connect',
                params: this.params
            });
        }

        /**
         * Handle WebSocket close
         */
        handleClose(event) {
            this.state = State.DISCONNECTED;
            this.stopHeartbeat();
            // The shadow is only valid for the connection that produced it.
            this.lastHtml = null;
            this.emit('stateChanged', State.DISCONNECTED);
            this.emit('close', event);

            if (!event.wasClean) {
                this.scheduleReconnect();
            }
        }

        /**
         * Handle WebSocket error
         */
        handleError(error) {
            console.error('WebSocket error:', error);
            this.emit('error', error);
        }

        /**
         * Handle incoming message
         */
        handleMessage(msg) {
            // Normalize type to lowercase for case-insensitive matching
            const type = (msg.type || '').toLowerCase();

            switch (type) {
                case 'render':
                    this.liveviewId = msg.liveview_id;
                    this.lastHtml = msg.html;
                    this.applyRender(msg.html);
                    this.emit('render', msg.html);
                    break;

                case 'patch':
                    this.applyPatch(msg.diff);
                    this.emit('patch', msg.diff);
                    break;

                case 'stream':
                    this.applyStream(msg.ops || []);
                    this.emit('stream', msg.ops);
                    break;

                case 'redirect':
                    window.location.href = msg.url;
                    break;

                case 'error':
                    console.error('LiveView error:', msg.message);
                    this.emit('error', msg.message);
                    break;

                case 'heartbeat_ack':
                case 'heartbeatack':
                    // Heartbeat acknowledged
                    break;

                default:
                    console.warn('Unknown message type:', msg.type);
            }
        }

        /**
         * Apply a full render by morphing the live region — node identity
         * (and with it focus and widget state) survives even full renders.
         */
        applyRender(html) {
            const root = this.getRoot();
            if (!root) return;
            morph(root, html);
            this.emit('morphed', root);
            this.bindEvents();
        }

        /**
         * Parse an HTML string for a stream op into a single element node,
         * inerting any scripts. Returns null on empty/invalid markup.
         */
        parseStreamNode(html) {
            const tpl = document.createElement('template');
            tpl.innerHTML = (html || '').trim();
            const node = tpl.content.firstElementChild;
            return node ? prepareIncoming(node) : null;
        }

        /**
         * Apply targeted collection ops (append/prepend/insert/remove/reset)
         * directly to a container by id — outside the diff shadow, so streamed
         * rows don't fight render patches. Re-adding an existing id moves it.
         */
        applyStream(ops) {
            for (const op of ops || []) {
                try {
                    if (op.op === 'remove') {
                        const el = document.getElementById(op.id);
                        if (el) el.remove();
                        continue;
                    }
                    if (op.op === 'reset') {
                        const c = document.getElementById(op.container);
                        if (c) c.replaceChildren();
                        continue;
                    }
                    const container = document.getElementById(op.container);
                    if (!container) continue;
                    const node = this.parseStreamNode(op.html);
                    if (!node) continue;
                    // De-dupe by id: drop any existing node so we re-insert once.
                    if (op.id) {
                        const existing = document.getElementById(op.id);
                        if (existing) existing.remove();
                    }
                    if (op.op === 'prepend') {
                        container.insertBefore(node, container.firstChild);
                    } else if (op.op === 'insert') {
                        const ref = op.before ? document.getElementById(op.before) : null;
                        container.insertBefore(node, ref); // null ref -> append
                    } else {
                        container.appendChild(node); // append (default)
                    }
                } catch (e) {
                    console.error('[LiveView] stream op failed:', e, op);
                }
            }
            // Bind soli-* handlers on freshly inserted nodes.
            this.bindEvents();
        }

        /**
         * Apply a patch: splice the shadow HTML string, then morph the DOM
         * to match. Any inconsistency falls back to a server resync.
         */
        applyPatch(patches) {
            if (typeof patches === 'string') {
                try {
                    patches = JSON.parse(patches);
                } catch (e) {
                    console.error('[LiveView] invalid patch payload:', e);
                    this.resync();
                    return;
                }
            }

            if (!Array.isArray(patches)) {
                this.resync();
                return;
            }
            if (patches.length === 0) return;

            for (const patch of patches) {
                if (patch.type === 'splice') {
                    if (this.lastHtml === null) {
                        this.resync();
                        return;
                    }
                    const next = spliceLines(this.lastHtml, patch.at, patch.del, patch.ins);
                    if (next === null) {
                        console.warn('[LiveView] splice did not apply, resyncing');
                        this.resync();
                        return;
                    }
                    this.lastHtml = next;
                } else if (patch.type === 'replace' && typeof patch.new === 'string' && !patch.old) {
                    this.lastHtml = patch.new;
                } else {
                    // Unknown patch shape (e.g. a legacy anchored replace
                    // from an older server) — recover via full render.
                    console.warn('[LiveView] unsupported patch, resyncing:', patch.type);
                    this.resync();
                    return;
                }
            }

            const root = this.getRoot();
            if (root) {
                morph(root, this.lastHtml);
                this.emit('morphed', root);
            }
            this.bindEvents();
        }

        /**
         * Ask the server to replay the last full render (recovery path when
         * the shadow copy is missing or a splice cannot apply).
         */
        resync() {
            this.lastHtml = null;
            this.send({
                type: 'resync',
                liveview_id: this.liveviewId
            });
        }

        /**
         * Bind click and form submission handlers (only once)
         */
        bindEvents() {
            // Only bind events once to prevent duplicate handlers
            if (this.eventsBound) return;
            this.eventsBound = true;

            const root = this.getRoot();

            // Helper: check if element belongs to this LiveView instance
            const owns = (el) => root.contains(el);

            // Click handlers - support both soli-click and data-soli-click
            document.addEventListener('click', (e) => {
                const btn = e.target.closest('[soli-click], [data-soli-click]');
                if (btn && owns(btn)) {
                    e.preventDefault();
                    const handler = btn.getAttribute('soli-click') || btn.getAttribute('data-soli-click');
                    this.sendEvent('click', handler, btn);
                }
            });

            // Form submission handlers - support both soli-submit and data-soli-submit
            document.addEventListener('submit', (e) => {
                const form = e.target.closest('[soli-submit], [data-soli-submit]');
                if (form && owns(form)) {
                    e.preventDefault();
                    const handler = form.getAttribute('soli-submit') || form.getAttribute('data-soli-submit');
                    this.sendEvent('submit', handler, form);
                }
            });

            // Input change handlers - support both soli-change and data-soli-change
            document.addEventListener('input', (e) => {
                const input = e.target.closest('[soli-change], [data-soli-change]');
                if (input && owns(input)) {
                    const handler = input.getAttribute('soli-change') || input.getAttribute('data-soli-change');
                    this.sendEvent('change', handler, input);
                }
            });

            // Focus/blur handlers
            document.addEventListener('blur', (e) => {
                const el = e.target.closest('[soli-blur], [data-soli-blur]');
                if (el && owns(el)) {
                    const handler = el.getAttribute('soli-blur') || el.getAttribute('data-soli-blur');
                    this.sendEvent('blur', handler, el);
                }
            }, true);

            document.addEventListener('focus', (e) => {
                const el = e.target.closest('[soli-focus], [data-soli-focus]');
                if (el && owns(el)) {
                    const handler = el.getAttribute('soli-focus') || el.getAttribute('data-soli-focus');
                    this.sendEvent('focus', handler, el);
                }
            }, true);

            // Keyboard handlers
            document.addEventListener('keydown', (e) => {
                const el = e.target.closest('[soli-keydown], [data-soli-keydown]');
                if (el && owns(el)) {
                    const handler = el.getAttribute('soli-keydown') || el.getAttribute('data-soli-keydown');
                    this.sendEvent('keydown', handler, el, {
                        key: e.key,
                        code: e.code,
                        shiftKey: e.shiftKey,
                        ctrlKey: e.ctrlKey,
                        altKey: e.altKey
                    });
                }
            });

            document.addEventListener('keyup', (e) => {
                const el = e.target.closest('[soli-keyup], [data-soli-keyup]');
                if (el && owns(el)) {
                    const handler = el.getAttribute('soli-keyup') || el.getAttribute('data-soli-keyup');
                    this.sendEvent('keyup', handler, el, {
                        key: e.key,
                        code: e.code
                    });
                }
            });
        }

        /**
         * Send an event to the server
         */
        sendEvent(eventType, handlerName, element, extraData = {}) {
            // Collect soli-value-* attributes (both with and without data- prefix)
            const values = {};
            for (const attr of element.attributes) {
                const name = attr.name.replace(/^data-/, ''); // Remove data- prefix if present
                if (name.startsWith('soli-value-')) {
                    const key = name.replace('soli-value-', '');
                    values[key] = attr.value;
                }
            }

            // Get target (both with and without data- prefix)
            const target = element.getAttribute('soli-target') ||
                           element.getAttribute('data-soli-target') ||
                           'live';

            // Get name and value for inputs
            const name = element.getAttribute('name') || null;
            const value = element.value !== undefined ? element.value : null;

            this.send({
                type: 'event',
                event: handlerName,
                liveview_id: this.liveviewId,
                params: {
                    ...values,
                    ...extraData,
                    ...(name && { name }),
                    ...(value !== null && { value })
                },
                target: target
            });
        }

        /**
         * Send a message to the server
         */
        send(message) {
            if (this.socket && this.socket.readyState === WebSocket.OPEN) {
                this.socket.send(JSON.stringify(message));
            } else {
                console.warn('Socket not connected, message not sent');
            }
        }

        /**
         * Start heartbeat interval
         */
        startHeartbeat() {
            this.stopHeartbeat();
            this.heartbeatInterval = setInterval(() => {
                this.send({ type: 'heartbeat' });
            }, 30000); // 30 seconds
        }

        /**
         * Stop heartbeat interval
         */
        stopHeartbeat() {
            if (this.heartbeatInterval) {
                clearInterval(this.heartbeatInterval);
                this.heartbeatInterval = null;
            }
        }

        /**
         * Schedule a reconnection attempt
         */
        scheduleReconnect() {
            if (this.reconnectAttempts >= this.maxReconnectAttempts) {
                console.error('Max reconnection attempts reached');
                this.emit('error', 'Failed to connect after ' + this.maxReconnectAttempts + ' attempts');
                return;
            }

            this.state = State.RECONNECTING;
            this.emit('stateChanged', State.RECONNECTING);

            const delay = this.reconnectDelay * Math.pow(2, this.reconnectAttempts);
            this.reconnectAttempts++;

            console.log('Reconnecting in ' + delay + 'ms (attempt ' + this.reconnectAttempts + ')');

            setTimeout(() => {
                this.connect();
            }, delay);
        }

        /**
         * Add an event listener
         */
        on(event, handler) {
            if (!this.eventHandlers[event]) {
                this.eventHandlers[event] = [];
            }
            this.eventHandlers[event].push(handler);
        }

        /**
         * Remove an event listener
         */
        off(event, handler) {
            if (this.eventHandlers[event]) {
                this.eventHandlers[event] = this.eventHandlers[event].filter(h => h !== handler);
            }
        }

        /**
         * Emit an event
         */
        emit(event, data) {
            if (this.eventHandlers[event]) {
                this.eventHandlers[event].forEach(handler => {
                    try {
                        handler(data);
                    } catch (e) {
                        console.error('Event handler error:', e);
                    }
                });
            }
        }
    }

    // Export
    global.SoliLiveView = SoliLiveView;

    // Expose the pure pieces for testing.
    global.SoliLiveView.morph = morph;
    global.SoliLiveView.spliceLines = spliceLines;

    // Track all LiveView instances
    global.SoliLiveView.instances = [];

    // Convenience function to create and connect
    global.live = function(socketUrl, params) {
        const lv = new SoliLiveView(socketUrl, params);
        global.SoliLiveView.instances.push(lv);
        lv.connect();
        return lv;
    };

    // Auto-connect for elements with data-liveview-url attribute
    if (typeof document !== 'undefined') {
        document.addEventListener('DOMContentLoaded', function() {
            const elements = document.querySelectorAll('[data-liveview-url]');
            elements.forEach(function(el) {
                if (el.hasAttribute('data-liveview-manual')) return;
                let url = el.getAttribute('data-liveview-url');
                // Build proper WebSocket URL if relative path
                if (url.startsWith('/')) {
                    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
                    url = protocol + '//' + location.host + url;
                }
                global.live(url, { rootElement: el });
            });
        });
    }

    // Global LiveView connector (for script tag usage)
    global.LiveView = {
        connect: function() {
            const liveviewEl = document.querySelector('[data-liveview-url]');
            if (liveviewEl) {
                const url = liveviewEl.getAttribute('data-liveview-url');
                return global.live(url);
            }
            console.warn('No element with data-liveview-url attribute found');
            return null;
        }
    };

})(typeof window !== 'undefined' ? window : global);
