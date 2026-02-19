/**
 * SoliLang LiveView Client
 * 
 * Minimal client-side library for LiveView communication.
 * ~2KB uncompressed
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
                    this.applyRender(msg.html);
                    this.emit('render', msg.html);
                    break;

                case 'patch':
                    this.applyPatch(msg.diff);
                    this.emit('patch', msg.diff);
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
         * Apply a full render
         */
        applyRender(html) {
            // Find the root element for this LiveView instance
            const root = this.getRoot();
            root.innerHTML = html;

            // Re-bind event handlers
            this.bindEvents();
        }

        /**
         * Apply a patch (quick-diff output)
         */
        applyPatch(patches) {
            // Parse JSON string if needed
            if (typeof patches === 'string') {
                try {
                    patches = JSON.parse(patches);
                } catch (e) {
                    // If not valid JSON, treat as full HTML replacement
                    this.applyRender(patches);
                    return;
                }
            }

            if (!Array.isArray(patches)) {
                // Full replacement
                this.applyRender(patches);
                return;
            }

            patches.forEach(patch => {
                switch (patch.type) {
                    case 'replace':
                        if (patch.old && patch.new) {
                            this.replaceElement(patch.old, patch.new);
                        } else if (patch.new) {
                            this.applyRender(patch.new);
                        }
                        break;

                    case 'add':
                        if (patch.new) {
                            this.applyRender(patch.new);
                        }
                        break;

                    case 'remove':
                        // For removals, we typically receive a full replacement
                        break;
                }
            });

            // Re-bind event handlers after patches
            this.bindEvents();
        }

        /**
         * Replace an element by its content
         */
        replaceElement(oldContent, newContent) {
            // Create temporary elements
            const oldTemp = document.createElement('div');
            oldTemp.innerHTML = oldContent;
            const oldEl = oldTemp.firstElementChild;

            const newTemp = document.createElement('div');
            newTemp.innerHTML = newContent;
            const newEl = newTemp.firstElementChild;

            // Find the element in the DOM
            if (oldEl) {
                const selector = this.getSelectorForElement(oldEl);
                const existingEl = document.querySelector(selector);

                if (existingEl) {
                    existingEl.outerHTML = newContent;
                } else {
                    // Fallback: replace by data-live-id
                    const liveId = oldEl.getAttribute('data-live-id');
                    if (liveId) {
                        const el = document.querySelector(`[data-live-id="${liveId}"]`);
                        if (el) {
                            el.outerHTML = newContent;
                        }
                    }
                }
            }
        }

        /**
         * Get a CSS selector for an element
         */
        getSelectorForElement(el) {
            if (el.id) {
                return '#' + el.id;
            }

            const path = [];
            let current = el;

            while (current && current !== document.body) {
                let selector = current.tagName.toLowerCase();

                if (current.className && typeof current.className === 'string') {
                    const classes = current.className.split(/\s+/).filter(c => c).join('.');
                    if (classes) {
                        selector += '.' + classes;
                    }
                }

                const parent = current.parentElement;
                if (parent) {
                    const siblings = parent.querySelectorAll(':scope > ' + selector);
                    if (siblings.length > 1) {
                        const index = Array.from(siblings).indexOf(current);
                        selector += ':nth-of-type(' + (index + 1) + ')';
                    }
                }

                path.unshift(selector);
                current = parent;
            }

            return path.join(' > ');
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
            console.log('Found', elements.length, 'LiveView elements');
            elements.forEach(function(el) {
                if (el.hasAttribute('data-liveview-manual')) return;
                let url = el.getAttribute('data-liveview-url');
                // Build proper WebSocket URL if relative path
                if (url.startsWith('/')) {
                    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
                    url = protocol + '//' + location.host + url;
                }
                console.log('Auto-connecting LiveView:', url);
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
