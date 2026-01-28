//! WebSocket-based live reload for browser auto-refresh.
//!
//! This module provides a WebSocket endpoint that browsers connect to
//! for receiving live reload signals. Falls back to SSE if WebSocket fails.

use std::time::Duration;

use bytes::Bytes;
use futures_util::SinkExt;
use futures_util::StreamExt;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header;
use hyper::{header::HeaderValue, Request, Response, StatusCode};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite;

/// Check if a request is a WebSocket upgrade request.
fn is_websocket_upgrade<B>(req: &Request<B>) -> bool {
    if let Some(header_value) = req.headers().get(header::UPGRADE) {
        return header_value == HeaderValue::from_static("websocket");
    }
    false
}

/// Handle WebSocket upgrade request for live reload.
pub async fn handle_live_reload_websocket(
    mut req: Request<Incoming>,
    mut reload_rx: broadcast::Receiver<()>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    // Check if this is a valid WebSocket upgrade request
    if !is_websocket_upgrade(&req) {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Full::new(Bytes::from("Not a WebSocket upgrade request")))
            .unwrap());
    }

    // Perform the WebSocket upgrade
    let (response, websocket) = match hyper_tungstenite::upgrade(&mut req, None) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("[LiveReload WS] Upgrade error: {}", e);
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::new(Bytes::from(format!(
                    "WebSocket upgrade error: {}",
                    e
                ))))
                .unwrap());
        }
    };

    // Spawn a task to handle the WebSocket connection
    tokio::spawn(async move {
        // Wait for the WebSocket handshake to complete
        let stream = match websocket.await {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("[LiveReload WS] WebSocket handshake error: {}", e);
                return;
            }
        };

        // Split into read/write
        let (mut sink, mut stream) = stream.split();

        // Send initial connection confirmation
        if sink
            .send(tungstenite::Message::Text("connected".to_string()))
            .await
            .is_err()
        {
            return;
        }

        // Create interval for keepalive pings
        let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
        ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Use select! to handle multiple events properly
        loop {
            tokio::select! {
                // Handle incoming messages from client (detect disconnects)
                msg = stream.next() => {
                    match msg {
                        Some(Ok(tungstenite::Message::Close(_))) => {
                            // Client closed connection
                            break;
                        }
                        Some(Ok(tungstenite::Message::Pong(_))) => {
                            // Pong received, connection is alive
                        }
                        Some(Ok(_)) => {
                            // Other message, ignore
                        }
                        Some(Err(_)) => {
                            // Error reading, connection likely closed
                            break;
                        }
                        None => {
                            // Stream ended
                            break;
                        }
                    }
                }

                // Handle reload signals
                result = reload_rx.recv() => {
                    match result {
                        Ok(()) => {
                            if sink.send(tungstenite::Message::Text("reload".to_string())).await.is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // Lagged behind, send reload anyway
                            if sink.send(tungstenite::Message::Text("reload".to_string())).await.is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }

                // Send keepalive ping
                _ = ping_interval.tick() => {
                    if sink.send(tungstenite::Message::Ping(vec![])).await.is_err() {
                        break;
                    }
                }
            }
        }

        // Send close frame
        let _ = sink.send(tungstenite::Message::Close(None)).await;
    });

    // Return the upgrade response
    Ok(response)
}

/// The client-side JavaScript for WebSocket-based live reload.
///
/// This script connects to the WebSocket endpoint and performs an async
/// content replacement when a reload signal is received, preserving scroll position.
/// Falls back to SSE if WebSocket fails.
pub const LIVE_RELOAD_SCRIPT: &str = r#"<script>
(function(){
    if (window.__livereload) return;
    window.__livereload = {
        connected: false,
        reconnecting: false,
        reloading: false
    };

    var retryDelay = 100;
    var maxRetryDelay = 5000;
    var retryCount = 0;
    var maxRetries = 10;
    var ws = null;
    var es = null;

    // Simple DOM morphing - only update what changed
    function morphChildren(oldParent, newParent) {
        var oldNodes = Array.from(oldParent.childNodes);
        var newNodes = Array.from(newParent.childNodes);

        // Update existing nodes and add new ones
        for (var i = 0; i < newNodes.length; i++) {
            var newNode = newNodes[i];
            var oldNode = oldNodes[i];

            if (!oldNode) {
                // New node - append it
                oldParent.appendChild(newNode.cloneNode(true));
            } else {
                morphNode(oldParent, oldNode, newNode);
            }
        }

        // Remove extra old nodes
        while (oldParent.childNodes.length > newNodes.length) {
            oldParent.removeChild(oldParent.lastChild);
        }
    }

    function morphNode(parent, oldNode, newNode) {
        // Quick check: if identical, skip entirely
        if (oldNode.nodeType === newNode.nodeType) {
            if (oldNode.nodeType === 3) {
                // Text node
                if (oldNode.textContent !== newNode.textContent) {
                    oldNode.textContent = newNode.textContent;
                }
                return;
            }
            if (oldNode.nodeType === 1 && oldNode.tagName === newNode.tagName) {
                // Same element - check if content is identical via outerHTML
                if (oldNode.outerHTML === newNode.outerHTML) {
                    return; // Completely identical, skip
                }
                // Different content - update attributes and recurse
                morphAttributes(oldNode, newNode);
                morphChildren(oldNode, newNode);
                return;
            }
        }
        // Different node types or tags - replace entirely
        parent.replaceChild(newNode.cloneNode(true), oldNode);
    }

    function morphAttributes(oldEl, newEl) {
        // Remove old attributes not in new
        var oldAttrs = Array.from(oldEl.attributes);
        for (var i = 0; i < oldAttrs.length; i++) {
            if (!newEl.hasAttribute(oldAttrs[i].name)) {
                oldEl.removeAttribute(oldAttrs[i].name);
            }
        }
        // Set new attributes
        var newAttrs = Array.from(newEl.attributes);
        for (var i = 0; i < newAttrs.length; i++) {
            if (oldEl.getAttribute(newAttrs[i].name) !== newAttrs[i].value) {
                oldEl.setAttribute(newAttrs[i].name, newAttrs[i].value);
            }
        }
    }

    function reconnect() {
        if (window.__livereload.reconnecting) return;
        retryCount++;
        if (retryCount > maxRetries) {
            console.log('[livereload] Max retries reached, stopping reconnection');
            return;
        }
        window.__livereload.reconnecting = true;
        retryDelay = Math.min(retryDelay * 2, maxRetryDelay);
        console.log('[livereload] Reconnecting in ' + retryDelay + 'ms (attempt ' + retryCount + '/' + maxRetries + ')');
        setTimeout(function() {
            window.__livereload.reconnecting = false;
            connect();
        }, retryDelay);
    }

    function asyncReload() {
        if (window.__livereload.reloading) return;
        window.__livereload.reloading = true;

        fetch(window.location.href, {
            headers: { 'X-Live-Reload': 'true' },
            cache: 'no-store'
        })
        .then(function(response) {
            if (!response.ok) throw new Error('HTTP ' + response.status);
            return response.text();
        })
        .then(function(html) {
            // Parse the new HTML
            var parser = new DOMParser();
            var newDoc = parser.parseFromString(html, 'text/html');
            var timestamp = Date.now();

            // Update stylesheets - replace old with new, using cache busting
            var oldStyles = Array.from(document.querySelectorAll('link[rel="stylesheet"]'));
            var newStyles = Array.from(newDoc.querySelectorAll('link[rel="stylesheet"]'));

            // Build map of new stylesheet hrefs (without query params)
            var newStyleHrefs = new Set();
            newStyles.forEach(function(s) {
                var href = s.getAttribute('href');
                if (href) newStyleHrefs.add(href.split('?')[0]);
            });

            // Remove old stylesheets not in new doc, update existing ones
            oldStyles.forEach(function(oldStyle) {
                var href = oldStyle.getAttribute('href');
                if (!href) return;
                var baseHref = href.split('?')[0];

                if (newStyleHrefs.has(baseHref)) {
                    // Update existing stylesheet with cache busting
                    oldStyle.href = baseHref + '?_lr=' + timestamp;
                } else {
                    // Remove stylesheet not in new doc
                    oldStyle.remove();
                }
            });

            // Add new stylesheets that don't exist yet
            var existingHrefs = new Set();
            document.querySelectorAll('link[rel="stylesheet"]').forEach(function(s) {
                var href = s.getAttribute('href');
                if (href) existingHrefs.add(href.split('?')[0]);
            });
            newStyles.forEach(function(newStyle) {
                var href = newStyle.getAttribute('href');
                if (href && !existingHrefs.has(href.split('?')[0])) {
                    var link = document.createElement('link');
                    link.rel = 'stylesheet';
                    link.href = href.split('?')[0] + '?_lr=' + timestamp;
                    document.head.appendChild(link);
                }
            });

            // Update inline styles - replace entirely instead of appending
            var oldInlineStyles = Array.from(document.querySelectorAll('style'));
            var newInlineStyles = Array.from(newDoc.querySelectorAll('style'));

            // Remove all non-livereload inline styles
            oldInlineStyles.forEach(function(s) {
                if (!s.textContent.includes('__livereload')) s.remove();
            });

            // Add new inline styles with marker to track them
            newInlineStyles.forEach(function(s) {
                if (!s.textContent.includes('__livereload')) {
                    var clone = s.cloneNode(true);
                    clone.setAttribute('data-lr-style', 'true');
                    document.head.appendChild(clone);
                }
            });

            // Morph body content - only update what changed
            var newBody = newDoc.body;
            if (newBody) {
                morphChildren(document.body, newBody);
            }

            // Handle scripts - only re-execute scripts that are in the new document
            // Use a simpler approach: just update inline scripts in place
            var newBodyScripts = newDoc.body.querySelectorAll('script');
            var oldBodyScripts = document.body.querySelectorAll('script');

            // For inline scripts, the morphChildren already handled the DOM
            // We need to re-execute them by replacing with new script elements
            oldBodyScripts.forEach(function(oldScript) {
                if (oldScript.textContent.includes('__livereload')) return;
                if (oldScript.src) return; // Skip external scripts - they'll be handled separately

                // Re-execute inline script by replacing it
                var newScript = document.createElement('script');
                Array.from(oldScript.attributes).forEach(function(attr) {
                    newScript.setAttribute(attr.name, attr.value);
                });
                newScript.textContent = oldScript.textContent;
                oldScript.parentNode.replaceChild(newScript, oldScript);
            });

            // Update title if changed
            if (newDoc.title) {
                document.title = newDoc.title;
            }

            // Re-initialize common libraries after DOM update
            if (typeof hljs !== 'undefined') {
                hljs.highlightAll();
            }
            if (typeof Prism !== 'undefined') {
                Prism.highlightAll();
            }

            // Dispatch event for custom re-initialization
            document.dispatchEvent(new CustomEvent('livereload:update'));

            console.log('[livereload] Content updated');
            window.__livereload.reloading = false;
        })
        .catch(function(error) {
            console.log('[livereload] Async reload failed, doing full reload:', error);
            window.__livereload.reloading = false;
            location.reload();
        });
    }

    function closeExisting() {
        // Close any existing WebSocket connection
        if (ws) {
            try {
                ws.onclose = null; // Prevent reconnect loop
                ws.onerror = null;
                ws.close();
            } catch(e) {}
            ws = null;
        }
        // Close any existing EventSource connection
        if (es) {
            try {
                es.onerror = null;
                es.close();
            } catch(e) {}
            es = null;
        }
    }

    function connect() {
        // Close existing connections first to prevent accumulation
        closeExisting();

        // Try WebSocket first
        var protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        var wsUrl = protocol + '//' + window.location.host + '/__livereload_ws';

        try {
            ws = new WebSocket(wsUrl);
            ws.__livereload = true;
        } catch (e) {
            console.log('[livereload] WebSocket failed, falling back to SSE');
            connectSSE();
            return;
        }

        ws.onopen = function() {
            window.__livereload.connected = true;
            window.__livereload.reconnecting = false;
            retryDelay = 100;
            retryCount = 0; // Reset retry count on successful connection
            console.log('[livereload] Connected via WebSocket');
        };

        ws.onmessage = function(event) {
            if (event.data === 'reload') {
                console.log('[livereload] Reload signal received');
                asyncReload();
            } else if (event.data === 'ping') {
                // Keepalive response if needed
            }
        };

        ws.onerror = function(error) {
            console.log('[livereload] WebSocket error, falling back to SSE');
            closeExisting();
            connectSSE();
        };

        ws.onclose = function() {
            if (window.__livereload.connected) {
                window.__livereload.connected = false;
                ws = null; // Clear reference
                reconnect();
            }
        };
    }

    function connectSSE() {
        // Close existing connections first
        closeExisting();

        // Fallback to Server-Sent Events
        es = new EventSource('/__livereload');

        es.addEventListener('reload', function() {
            asyncReload();
        });

        es.onerror = function() {
            es.close();
            es = null;
            reconnect();
        };
    }

    // Clean up on page unload
    window.addEventListener('beforeunload', function() {
        if (ws) {
            try { ws.close(); } catch(e) {}
        }
        if (es) {
            try { es.close(); } catch(e) {}
        }
    });

    // Start connection
    connect();
})();
</script>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_websocket_check() {
        let req = Request::builder()
            .header(header::UPGRADE, "websocket")
            .header(header::CONNECTION, "upgrade")
            .body(())
            .unwrap();

        assert!(is_websocket_upgrade(&req));
    }

    #[test]
    fn test_script_contains_websocket_url() {
        assert!(LIVE_RELOAD_SCRIPT.contains("/__livereload_ws"));
    }

    #[test]
    fn test_script_contains_sse_fallback() {
        assert!(LIVE_RELOAD_SCRIPT.contains("/__livereload"));
    }

    #[test]
    fn test_script_has_cleanup() {
        assert!(LIVE_RELOAD_SCRIPT.contains("beforeunload"));
    }
}
