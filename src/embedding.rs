//! Embedding generation and vector similarity search.
//!
//! The embedding API is feature-gated behind `embedding` (enabled by default).
//! Configure via environment:
//!   SOLI_EMBEDDING_API_KEY=sk-...
//!   SOLI_EMBEDDING_URL=https://api.openai.com/v1/embeddings (default)
//!   SOLI_EMBEDDING_MODEL=text-embedding-3-small (default)

/// Generate an embedding vector for the given text by calling an external API.
/// Returns None if embedding API is not configured or the call fails.
#[cfg(feature = "embedding")]
pub fn generate_embedding(text: &str) -> Option<Vec<f64>> {
    let api_key = std::env::var("SOLI_EMBEDDING_API_KEY").ok()?;
    let url = std::env::var("SOLI_EMBEDDING_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1/embeddings".to_string());
    let model = std::env::var("SOLI_EMBEDDING_MODEL")
        .unwrap_or_else(|_| "text-embedding-3-small".to_string());

    let body = serde_json::json!({
        "input": text,
        "model": model,
    });

    let response = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_json(&body)
        .ok()?;

    let json: serde_json::Value = response.into_json().ok()?;
    let embedding = json["data"][0]["embedding"]
        .as_array()?
        .iter()
        .filter_map(|v| v.as_f64())
        .collect::<Vec<f64>>();

    if embedding.is_empty() {
        None
    } else {
        Some(embedding)
    }
}

#[cfg(not(feature = "embedding"))]
pub fn generate_embedding(_text: &str) -> Option<Vec<f64>> {
    None
}

/// Compute cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

/// Score a document against a query embedding by comparing the field value.
/// The field can be a JSON array of floats (embedding vector) or other types.
pub fn score_against_embedding(query_vec: &[f64], field_value: &serde_json::Value) -> f64 {
    match field_value {
        serde_json::Value::Array(arr) => {
            let doc_vec: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
            if doc_vec.len() == query_vec.len() {
                cosine_similarity(query_vec, &doc_vec)
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}
