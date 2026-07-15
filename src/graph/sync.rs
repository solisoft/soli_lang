//! Embed graph nodes and write the [`ProjectGraph`] into SolidB.
//!
//! Storage is a clean rebuild: the `soli_graph_nodes` and `soli_graph_edges`
//! collections are dropped and recreated on every run, so the graph always
//! reflects the current source. Connection + auth reuse the model layer's
//! [`db_config`] (the same SolidB the app's Models talk to), overridable with a
//! target database name.

use std::collections::HashMap;

use crate::graph::model::{ProjectGraph, EDGE_COLLECTION, NODE_COLLECTION, VECTOR_INDEX};
use crate::interpreter::builtins::model::db_config;
use crate::solidb_http::SoliDBClient;

/// Embed at most this many node texts per embedding-API request, so a large
/// project doesn't blow the provider's per-request input/token limits.
const EMBED_CHUNK: usize = 96;
/// Insert at most this many documents per bulk-insert AQL request.
const INSERT_CHUNK: usize = 200;

pub struct SyncOptions {
    /// Target database (defaults to `SOLIDB_DATABASE` / `default`).
    pub database: Option<String>,
    /// Whether nodes were embedded (drives vector-index creation).
    pub embed: bool,
}

pub struct SyncReport {
    pub nodes: usize,
    pub edges: usize,
    pub database: String,
    pub embedded: bool,
    /// Embedding dimension when `embedded`, else 0.
    pub dimension: usize,
}

/// Fill `node.embedding` for every node via the batch embedding API. Returns
/// the embedding dimension. Errors (with an actionable message) when the
/// embedding endpoint is unconfigured or unreachable — the caller offers
/// `--no-embed` as the escape hatch.
pub fn embed_graph(graph: &mut ProjectGraph) -> Result<usize, String> {
    if graph.nodes.is_empty() {
        return Ok(0);
    }
    let texts: Vec<String> = graph.nodes.iter().map(|n| n.text.clone()).collect();
    let mut vectors: Vec<Vec<f64>> = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(EMBED_CHUNK) {
        let part = crate::embedding::generate_embeddings_batch(chunk).ok_or_else(|| {
            "Embedding failed. Set SOLI_EMBEDDING_API_KEY (and SOLI_EMBEDDING_URL / \
             SOLI_EMBEDDING_MODEL for non-OpenAI providers), or re-run with --no-embed."
                .to_string()
        })?;
        vectors.extend(part);
    }
    if vectors.len() != graph.nodes.len() {
        return Err(format!(
            "Embedding returned {} vectors for {} nodes",
            vectors.len(),
            graph.nodes.len()
        ));
    }
    let dim = vectors.first().map(|v| v.len()).unwrap_or(0);
    for (node, vector) in graph.nodes.iter_mut().zip(vectors) {
        node.embedding = vector;
    }
    Ok(dim)
}

/// Write the graph into SolidB (drop + recreate collections, indexes, bulk
/// insert). Assumes embeddings are already attached when `opts.embed`.
pub fn write_graph(graph: &ProjectGraph, opts: &SyncOptions) -> Result<SyncReport, String> {
    let (client, database) = connect(opts)?;

    // Clean rebuild. Drops are best-effort (404 on first run is fine).
    let _ = client.drop_collection(NODE_COLLECTION);
    let _ = client.drop_collection(EDGE_COLLECTION);
    client
        .create_collection(NODE_COLLECTION, None)
        .map_err(|e| format!("create collection {}: {}", NODE_COLLECTION, e))?;
    client
        .create_collection(EDGE_COLLECTION, Some("edge"))
        .map_err(|e| format!("create edge collection {}: {}", EDGE_COLLECTION, e))?;

    // Vector index over node embeddings (dimension inferred from the data).
    let dimension = graph
        .nodes
        .iter()
        .map(|n| n.embedding.len())
        .find(|&d| d > 0)
        .unwrap_or(0);
    if opts.embed && dimension > 0 {
        client
            .create_vector_index(
                NODE_COLLECTION,
                VECTOR_INDEX,
                "embedding",
                dimension,
                "cosine",
                None,
            )
            .map_err(|e| format!("create vector index: {}", e))?;
    }

    // Traversal + filter indexes. Best-effort — never fatal.
    let _ = client.create_index(
        EDGE_COLLECTION,
        "edge_from",
        vec!["_from".to_string()],
        false,
        "hash",
    );
    let _ = client.create_index(
        EDGE_COLLECTION,
        "edge_to",
        vec!["_to".to_string()],
        false,
        "hash",
    );
    let _ = client.create_index(
        EDGE_COLLECTION,
        "edge_kind",
        vec!["edge_kind".to_string()],
        false,
        "hash",
    );
    let _ = client.create_index(
        NODE_COLLECTION,
        "node_kind",
        vec!["kind".to_string()],
        false,
        "hash",
    );

    let node_docs: Vec<serde_json::Value> = graph
        .nodes
        .iter()
        .map(ProjectGraph::node_document)
        .collect();
    bulk_insert(&client, NODE_COLLECTION, node_docs)?;

    let edge_docs: Vec<serde_json::Value> = graph
        .edges
        .iter()
        .map(ProjectGraph::edge_document)
        .collect();
    bulk_insert(&client, EDGE_COLLECTION, edge_docs)?;

    Ok(SyncReport {
        nodes: graph.nodes.len(),
        edges: graph.edges.len(),
        database,
        embedded: opts.embed && dimension > 0,
        dimension: if opts.embed { dimension } else { 0 },
    })
}

/// Bulk-insert documents via `FOR d IN @docs INSERT d INTO <collection>`,
/// chunked so no single request carries the whole (embedding-heavy) payload.
fn bulk_insert(
    client: &SoliDBClient,
    collection: &str,
    docs: Vec<serde_json::Value>,
) -> Result<(), String> {
    let query = format!("FOR d IN @docs INSERT d INTO {}", collection);
    for chunk in docs.chunks(INSERT_CHUNK) {
        let mut bind = HashMap::new();
        bind.insert("docs".to_string(), serde_json::Value::Array(chunk.to_vec()));
        client
            .query(&query, Some(bind))
            .map_err(|e| format!("bulk insert into {}: {}", collection, e))?;
    }
    Ok(())
}

/// Build a SolidB client from the model-layer config, honoring an optional
/// database override. Auth priority mirrors `crud.rs`: JWT > API key > basic.
fn connect(opts: &SyncOptions) -> Result<(SoliDBClient, String), String> {
    let host = format!(
        "{}{}",
        db_config::DB_CONFIG.scheme,
        db_config::DB_CONFIG.host
    );
    let database = opts
        .database
        .clone()
        .unwrap_or_else(db_config::get_database_name);
    let mut client =
        SoliDBClient::connect(&host).map_err(|e| format!("connect to {}: {}", host, e))?;
    client.set_database(&database);
    if let Some(jwt) = db_config::get_jwt_token() {
        client = client.with_jwt_token(&jwt);
    } else if let Some(key) = db_config::get_api_key() {
        client = client.with_api_key(key);
    } else if let (Ok(user), Ok(pass)) = (
        std::env::var("SOLIDB_USERNAME"),
        std::env::var("SOLIDB_PASSWORD"),
    ) {
        client = client.with_basic_auth(&user, &pass);
    }
    Ok((client, database))
}
