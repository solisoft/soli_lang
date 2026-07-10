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
