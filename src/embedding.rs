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

/// Embed several texts in a single `/v1/embeddings` request (the API accepts an
/// array `input`). Much faster than N calls to `generate_embedding` when
/// back-filling embeddings over an existing collection. Returns one vector per
/// input, in input order; `None` if not configured or the call fails.
#[cfg(feature = "embedding")]
pub fn generate_embeddings_batch(texts: &[String]) -> Option<Vec<Vec<f64>>> {
    if texts.is_empty() {
        return Some(Vec::new());
    }

    let api_key = std::env::var("SOLI_EMBEDDING_API_KEY").ok()?;
    let url = std::env::var("SOLI_EMBEDDING_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1/embeddings".to_string());
    let model = std::env::var("SOLI_EMBEDDING_MODEL")
        .unwrap_or_else(|_| "text-embedding-3-small".to_string());

    let body = serde_json::json!({
        "input": texts,
        "model": model,
    });

    let response = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_json(&body)
        .ok()?;

    let json: serde_json::Value = response.into_json().ok()?;
    let data = json["data"].as_array()?;

    // OpenAI returns each entry with an `index`; re-order by it so the result
    // aligns with the input even if the server returns them out of order.
    let mut indexed: Vec<(usize, Vec<f64>)> = Vec::with_capacity(data.len());
    for (position, item) in data.iter().enumerate() {
        let index = item["index"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(position);
        let vector: Vec<f64> = item["embedding"]
            .as_array()?
            .iter()
            .filter_map(|v| v.as_f64())
            .collect();
        indexed.push((index, vector));
    }
    indexed.sort_by_key(|(index, _)| *index);

    let vectors: Vec<Vec<f64>> = indexed.into_iter().map(|(_, vector)| vector).collect();
    if vectors.len() != texts.len() || vectors.iter().any(|v| v.is_empty()) {
        return None;
    }
    Some(vectors)
}

#[cfg(not(feature = "embedding"))]
pub fn generate_embeddings_batch(_texts: &[String]) -> Option<Vec<Vec<f64>>> {
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
