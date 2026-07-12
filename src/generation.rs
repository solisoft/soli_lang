//! LLM text completion via an OpenAI-compatible `chat/completions` endpoint.
//!
//! The completion API is feature-gated behind `llm` (enabled by default).
//! Configure via environment:
//!   SOLI_LLM_URL=https://api.openai.com/v1/chat/completions (default)
//!   SOLI_LLM_API_KEY=sk-...        (optional — local vLLM / Ollama / llama.cpp
//!                                   servers often don't require one)
//!   SOLI_LLM_MODEL=gpt-4o-mini     (default)
//!   SOLI_LLM_TEMPERATURE=0.7       (optional)
//!   SOLI_LLM_MAX_TOKENS=1024       (optional)
//!
//! Centralising the endpoint + key here (rather than in application Soli code)
//! keeps credentials out of the app and gives one place to reason about where
//! prompts are sent — a single point for GDPR / data-residency review.

/// Generate a chat completion from a `system` + `user` prompt by POSTing to an
/// external OpenAI-compatible endpoint. Returns `None` if the endpoint is
/// unreachable, the request fails, or the response has no completion text.
#[cfg(feature = "llm")]
pub fn generate_completion(system: &str, user: &str) -> Option<String> {
    let url = std::env::var("SOLI_LLM_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());
    let model = std::env::var("SOLI_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    let mut body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });

    // Optional generation controls — only sent when explicitly configured so
    // servers keep their own defaults otherwise.
    if let Some(temperature) = std::env::var("SOLI_LLM_TEMPERATURE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        body["temperature"] = serde_json::json!(temperature);
    }
    if let Some(max_tokens) = std::env::var("SOLI_LLM_MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        body["max_tokens"] = serde_json::json!(max_tokens);
    }

    let mut request = ureq::post(&url).set("Content-Type", "application/json");
    // The API key is optional: many self-hosted OpenAI-compatible servers
    // accept unauthenticated requests. Only attach the header when set.
    if let Ok(api_key) = std::env::var("SOLI_LLM_API_KEY") {
        if !api_key.is_empty() {
            request = request.set("Authorization", &format!("Bearer {}", api_key));
        }
    }

    let response = request.send_json(&body).ok()?;
    let json: serde_json::Value = response.into_json().ok()?;
    let content = json["choices"][0]["message"]["content"].as_str()?;
    Some(content.to_string())
}

#[cfg(not(feature = "llm"))]
pub fn generate_completion(_system: &str, _user: &str) -> Option<String> {
    None
}

/// Stream a chat completion token-by-token from an OpenAI-compatible endpoint
/// (`stream: true`). `on_token` is invoked with each content delta as it
/// arrives and returns `false` to stop early (e.g. the client disconnected);
/// the accumulated full text is returned, or `None` if the request fails.
/// Blocking — call it from a worker thread (an `sse`/`stream` block).
#[cfg(feature = "llm")]
pub fn generate_completion_stream<F: FnMut(&str) -> bool>(
    system: &str,
    user: &str,
    mut on_token: F,
) -> Option<String> {
    use std::io::{BufRead, BufReader};

    let url = std::env::var("SOLI_LLM_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());
    let model = std::env::var("SOLI_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    let mut body = serde_json::json!({
        "model": model,
        "stream": true,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    if let Some(temperature) = std::env::var("SOLI_LLM_TEMPERATURE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        body["temperature"] = serde_json::json!(temperature);
    }
    if let Some(max_tokens) = std::env::var("SOLI_LLM_MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        body["max_tokens"] = serde_json::json!(max_tokens);
    }

    let mut request = ureq::post(&url).set("Content-Type", "application/json");
    if let Ok(api_key) = std::env::var("SOLI_LLM_API_KEY") {
        if !api_key.is_empty() {
            request = request.set("Authorization", &format!("Bearer {}", api_key));
        }
    }

    let response = request.send_json(&body).ok()?;
    let reader = BufReader::new(response.into_reader());
    let mut full = String::new();
    // OpenAI streams `data: {json}\n\n` frames, ending with `data: [DONE]`.
    for line in reader.lines() {
        let Ok(line) = line else { break };
        match parse_sse_delta(&line) {
            SseDelta::Token(tok) => {
                full.push_str(&tok);
                if !on_token(&tok) {
                    break; // consumer asked to stop (client gone)
                }
            }
            SseDelta::Done => break,
            SseDelta::Skip => {}
        }
    }
    Some(full)
}

/// The meaning of one line of an OpenAI streaming (`data:`) response.
#[cfg_attr(not(feature = "llm"), allow(dead_code))]
enum SseDelta {
    Token(String),
    Done,
    Skip,
}

/// Parse a single SSE line from a chat-completions stream into a content token,
/// the terminal `[DONE]` marker, or something to skip (keepalives, blank lines,
/// deltas without content). Pure — unit-testable without a network call.
#[cfg_attr(not(feature = "llm"), allow(dead_code))]
fn parse_sse_delta(line: &str) -> SseDelta {
    let Some(data) = line.trim().strip_prefix("data:") else {
        return SseDelta::Skip;
    };
    let data = data.trim();
    if data == "[DONE]" {
        return SseDelta::Done;
    }
    match serde_json::from_str::<serde_json::Value>(data) {
        Ok(json) => match json["choices"][0]["delta"]["content"].as_str() {
            Some(tok) if !tok.is_empty() => SseDelta::Token(tok.to_string()),
            _ => SseDelta::Skip,
        },
        Err(_) => SseDelta::Skip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sse_delta_extracts_tokens_and_terminators() {
        assert!(matches!(
            parse_sse_delta(r#"data: {"choices":[{"delta":{"content":"Hel"}}]}"#),
            SseDelta::Token(t) if t == "Hel"
        ));
        assert!(matches!(parse_sse_delta("data: [DONE]"), SseDelta::Done));
        // role-only first delta, blank lines, and non-data lines are skipped
        assert!(matches!(
            parse_sse_delta(r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#),
            SseDelta::Skip
        ));
        assert!(matches!(parse_sse_delta(""), SseDelta::Skip));
        assert!(matches!(parse_sse_delta(": keepalive"), SseDelta::Skip));
        assert!(matches!(parse_sse_delta("data: not-json"), SseDelta::Skip));
    }
}

#[cfg(not(feature = "llm"))]
pub fn generate_completion_stream<F: FnMut(&str) -> bool>(
    _system: &str,
    _user: &str,
    _on_token: F,
) -> Option<String> {
    None
}
