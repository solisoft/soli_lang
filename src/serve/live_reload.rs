//! Live reload SSE endpoint for browser auto-refresh.
//!
//! This module provides Server-Sent Events (SSE) functionality that allows
//! browsers to automatically refresh when file changes are detected during
//! development.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use tokio::sync::broadcast;

/// Global flag indicating whether live reload is enabled.
static LIVE_RELOAD_ENABLED: AtomicBool = AtomicBool::new(false);

/// Set whether live reload is enabled.
pub fn set_live_reload_enabled(enabled: bool) {
    LIVE_RELOAD_ENABLED.store(enabled, Ordering::SeqCst);
}

/// Check if live reload is enabled.
pub fn is_live_reload_enabled() -> bool {
    LIVE_RELOAD_ENABLED.load(Ordering::SeqCst)
}

/// Handle a live reload SSE connection.
///
/// This function waits for a reload signal on the broadcast channel and sends
/// an SSE event to the browser when triggered. If no signal is received within
/// the timeout period, it sends a keepalive comment to maintain the connection.
///
/// The browser's EventSource API will automatically reconnect after receiving
/// a response, creating a long-polling effect.
pub async fn handle_live_reload_sse(
    mut reload_rx: broadcast::Receiver<()>,
) -> Response<Full<Bytes>> {
    match tokio::time::timeout(Duration::from_secs(10), reload_rx.recv()).await {
        Ok(Ok(())) => {
            // Reload signal received - send reload event
            Response::builder()
                .status(200)
                .header("Content-Type", "text/event-stream")
                .header("Cache-Control", "no-cache")
                .header("Connection", "keep-alive")
                .header("Access-Control-Allow-Origin", "*")
                .body(Full::new(Bytes::from("event: reload\ndata: reload\n\n")))
                .unwrap()
        }
        Ok(Err(_)) => {
            // Channel closed - server shutting down
            Response::builder()
                .status(200)
                .header("Content-Type", "text/event-stream")
                .header("Cache-Control", "no-cache")
                .body(Full::new(Bytes::from(": server closing\n\n")))
                .unwrap()
        }
        Err(_) => {
            // Timeout - send keepalive comment, browser will reconnect
            Response::builder()
                .status(200)
                .header("Content-Type", "text/event-stream")
                .header("Cache-Control", "no-cache")
                .header("Connection", "keep-alive")
                .header("Access-Control-Allow-Origin", "*")
                .body(Full::new(Bytes::from(": keepalive\n\n")))
                .unwrap()
        }
    }
}

/// The JavaScript snippet that gets injected into HTML responses.
///
/// This creates an EventSource connection to the `/__livereload` endpoint
/// and reloads the page when a reload event is received.
pub const LIVE_RELOAD_SCRIPT: &str = r#"<script>
(function(){
    if (window.__livereload_script_injected) return;
    window.__livereload_script_injected = true;
    var es = new EventSource('/__livereload');
    es.addEventListener('reload', function() {
        location.reload();
    });
    es.onerror = function() {
        setTimeout(function() {
            es.close();
            es = new EventSource('/__livereload');
        }, 1000);
    };
    window.addEventListener('beforeunload', function() {
        es.close();
    });
    document.addEventListener('click', function(e) {
        var link = e.target.closest('a');
        if (link && link.href && link.href.startsWith(location.origin)) {
            es.close();
        }
    });
})();
</script>"#;

/// Find the last occurrence of an ASCII pattern in a string, case-insensitive.
/// Does NOT allocate any intermediate strings - works directly on bytes.
/// Only valid for ASCII needles (like HTML tags).
#[inline]
fn rfind_ascii_case_insensitive(haystack: &str, needle: &[u8]) -> Option<usize> {
    let haystack_bytes = haystack.as_bytes();
    let needle_len = needle.len();

    if haystack_bytes.len() < needle_len {
        return None;
    }

    // Search from the end
    for i in (0..=(haystack_bytes.len() - needle_len)).rev() {
        let mut matches = true;
        for j in 0..needle_len {
            if haystack_bytes[i + j].to_ascii_lowercase() != needle[j] {
                matches = false;
                break;
            }
        }
        if matches {
            return Some(i);
        }
    }
    None
}

/// Inject the live reload script into HTML content.
///
/// Inserts the script before the closing `</body>` tag if present,
/// otherwise appends it to the end of the HTML.
pub fn inject_live_reload_script(html: &str) -> String {
    // Skip if already injected
    if html.contains("__livereload_script_injected") {
        return html.to_string();
    }

    // Try to find </body> (case-insensitive) without allocating a lowercase copy
    // Using byte slices for ASCII HTML tags
    if let Some(pos) = rfind_ascii_case_insensitive(html, b"</body>") {
        let mut result = String::with_capacity(html.len() + LIVE_RELOAD_SCRIPT.len());
        result.push_str(&html[..pos]);
        result.push_str(LIVE_RELOAD_SCRIPT);
        result.push_str(&html[pos..]);
        result
    } else if let Some(pos) = rfind_ascii_case_insensitive(html, b"</html>") {
        // Fallback: insert before </html>
        let mut result = String::with_capacity(html.len() + LIVE_RELOAD_SCRIPT.len());
        result.push_str(&html[..pos]);
        result.push_str(LIVE_RELOAD_SCRIPT);
        result.push_str(&html[pos..]);
        result
    } else {
        // Last resort: append at the end
        format!("{}{}", html, LIVE_RELOAD_SCRIPT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_before_body() {
        let html = "<html><body><h1>Hello</h1></body></html>";
        let result = inject_live_reload_script(html);
        assert!(result.contains(LIVE_RELOAD_SCRIPT));
        assert!(result.contains("<h1>Hello</h1>"));
        // Script should be before </body>
        let script_pos = result.find(LIVE_RELOAD_SCRIPT).unwrap();
        let body_pos = result.find("</body>").unwrap();
        assert!(script_pos < body_pos);
    }

    #[test]
    fn test_inject_case_insensitive() {
        let html = "<HTML><BODY><h1>Hello</h1></BODY></HTML>";
        let result = inject_live_reload_script(html);
        assert!(result.contains(LIVE_RELOAD_SCRIPT));
    }

    #[test]
    fn test_inject_no_body_tag() {
        let html = "<html><h1>Hello</h1></html>";
        let result = inject_live_reload_script(html);
        assert!(result.contains(LIVE_RELOAD_SCRIPT));
        // Script should be before </html>
        let script_pos = result.find(LIVE_RELOAD_SCRIPT).unwrap();
        let html_pos = result.find("</html>").unwrap();
        assert!(script_pos < html_pos);
    }

    #[test]
    fn test_inject_minimal_html() {
        let html = "<h1>Hello</h1>";
        let result = inject_live_reload_script(html);
        assert!(result.contains(LIVE_RELOAD_SCRIPT));
        assert!(result.ends_with("</script>"));
    }
}
