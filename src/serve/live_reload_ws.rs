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
/// This script connects to the WebSocket endpoint and reloads the page
/// when a reload signal is received. Falls back to SSE if WebSocket fails.
pub const LIVE_RELOAD_SCRIPT: &str = r#"<script>
(function(){
    if (window.__livereload) return;
    window.__livereload = {
        connected: false,
        reconnecting: false
    };

    var retryDelay = 100;
    var maxRetryDelay = 5000;
    var ws = null;
    var es = null;

    function reconnect() {
        if (window.__livereload.reconnecting) return;
        window.__livereload.reconnecting = true;
        retryDelay = Math.min(retryDelay * 2, maxRetryDelay);
        setTimeout(function() {
            window.__livereload.reconnecting = false;
            connect();
        }, retryDelay);
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
                location.reload();
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
            location.reload();
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
