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
                .body(Full::new(Bytes::from(format!("WebSocket upgrade error: {}", e))))
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
        let (mut sink, mut _stream) = stream.split();

        // Create a channel for sending messages
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<tungstenite::Message, tungstenite::Error>>(32);

        // Send initial connection confirmation
        if let Err(e) = tx.send(Ok(tungstenite::Message::Text("connected".to_string()))).await {
            eprintln!("[LiveReload WS] Failed to send confirmation: {}", e);
            return;
        }

        // Spawn a task to forward messages from channel to WebSocket
        let forward_task = tokio::spawn(async move {
            while let Some(msg_result) = rx.recv().await {
                match msg_result {
                    Ok(msg) => {
                        if sink.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Listen for reload signals with keepalive
        loop {
            match tokio::time::timeout(Duration::from_secs(60), reload_rx.recv()).await {
                Ok(Ok(())) => {
                    // Reload signal received
                    if let Err(_) = tx.send(Ok(tungstenite::Message::Text("reload".to_string()))).await {
                        // Client disconnected - this is normal during page reload
                        break;
                    }
                }
                Ok(Err(_)) => {
                    // Channel closed
                    break;
                }
                Err(_) => {
                    // Timeout - send keepalive ping
                    if let Err(_) = tx.send(Ok(tungstenite::Message::Ping(vec![]))).await {
                        // Client disconnected
                        break;
                    }
                }
            }
        }

        // Cleanup
        forward_task.abort();
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
        window.__livereload.reconnecting = true;
        retryDelay = Math.min(retryDelay * 2, maxRetryDelay);
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

            // Update stylesheets - force reload all with cache busting
            var oldStyles = document.querySelectorAll('link[rel="stylesheet"]');
            var timestamp = Date.now();
            oldStyles.forEach(function(oldStyle) {
                var href = oldStyle.getAttribute('href');
                if (href) {
                    var baseHref = href.split('?')[0];
                    var newLink = document.createElement('link');
                    newLink.rel = 'stylesheet';
                    newLink.href = baseHref + '?_lr=' + timestamp;
                    newLink.onload = function() {
                        if (oldStyle.parentNode) oldStyle.parentNode.removeChild(oldStyle);
                    };
                    oldStyle.parentNode.insertBefore(newLink, oldStyle.nextSibling);
                }
            });

            // Update inline styles
            var newInlineStyles = newDoc.querySelectorAll('style');
            var oldInlineStyles = document.querySelectorAll('style');
            oldInlineStyles.forEach(function(s) {
                if (!s.textContent.includes('__livereload')) s.remove();
            });
            newInlineStyles.forEach(function(s) {
                document.head.appendChild(s.cloneNode(true));
            });

            // Morph body content - only update what changed
            var newBody = newDoc.body;
            if (newBody) {
                morphChildren(document.body, newBody);
            }

            // Re-execute all scripts in body (both inline and external)
            var scripts = document.body.querySelectorAll('script');
            var scriptsToLoad = [];
            scripts.forEach(function(oldScript) {
                if (oldScript.textContent.includes('__livereload')) return;

                var newScript = document.createElement('script');

                // Copy attributes
                Array.from(oldScript.attributes).forEach(function(attr) {
                    newScript.setAttribute(attr.name, attr.value);
                });

                if (oldScript.src) {
                    // External script - add cache busting and load sequentially
                    var baseSrc = oldScript.src.split('?')[0];
                    newScript.src = baseSrc + '?_lr=' + Date.now();
                    scriptsToLoad.push(newScript);
                } else {
                    // Inline script - execute immediately
                    newScript.textContent = oldScript.textContent;
                    oldScript.parentNode.replaceChild(newScript, oldScript);
                }
            });

            // Load external scripts sequentially
            function loadNextScript(index) {
                if (index >= scriptsToLoad.length) return;
                var script = scriptsToLoad[index];
                script.onload = function() { loadNextScript(index + 1); };
                script.onerror = function() { loadNextScript(index + 1); };
                document.body.appendChild(script);
            }
            loadNextScript(0);

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

    function connect() {
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
            try { ws.close(); } catch(e) {}
            connectSSE();
        };

        ws.onclose = function() {
            if (window.__livereload.connected) {
                window.__livereload.connected = false;
                reconnect();
            }
        };
    }

    function connectSSE() {
        // Fallback to Server-Sent Events
        es = new EventSource('/__livereload');

        es.addEventListener('reload', function() {
            asyncReload();
        });

        es.onerror = function() {
            es.close();
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
    use tokio::sync::broadcast;

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
