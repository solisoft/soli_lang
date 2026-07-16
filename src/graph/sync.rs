//! Embed graph nodes and write the [`ProjectGraph`] into SolidB.
//!
//! Storage is a clean rebuild: the `soli_graph_nodes` and `soli_graph_edges`
//! collections are dropped and recreated on every run, so the graph always
//! reflects the current source. Connection + auth reuse the model layer's
//! [`db_config`] (the same SolidB the app's Models talk to), overridable with a
//! target database name.

use std::collections::HashMap;
use std::path::Path;

use crate::graph::builder::RouteSnapshot;
use crate::graph::model::{
    ProjectGraph, EDGE_COLLECTION, META_COLLECTION, NODE_COLLECTION, VECTOR_INDEX,
};
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
pub fn embed_graph(
    graph: &mut ProjectGraph,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<usize, String> {
    if graph.nodes.is_empty() {
        return Ok(0);
    }
    let texts: Vec<String> = graph.nodes.iter().map(|n| n.text.clone()).collect();
    let total = texts.len();
    let mut vectors: Vec<Vec<f64>> = Vec::with_capacity(total);
    for chunk in texts.chunks(EMBED_CHUNK) {
        let part = crate::embedding::generate_embeddings_batch(chunk).ok_or_else(|| {
            "Embedding failed. Set SOLI_EMBEDDING_API_KEY (and SOLI_EMBEDDING_URL / \
             SOLI_EMBEDDING_MODEL for non-OpenAI providers), or re-run with --no-embed."
                .to_string()
        })?;
        vectors.extend(part);
        on_progress(vectors.len(), total);
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
/// `on_progress` reports inserted-document count as `(docs_done, total)`.
pub fn write_graph(
    graph: &ProjectGraph,
    opts: &SyncOptions,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<SyncReport, String> {
    let (client, database) = connect(opts.database.as_deref())?;

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
    let edge_docs: Vec<serde_json::Value> = graph
        .edges
        .iter()
        .map(ProjectGraph::edge_document)
        .collect();
    let total_docs = node_docs.len() + edge_docs.len();
    let mut inserted = 0usize;
    bulk_insert(&client, NODE_COLLECTION, node_docs, &mut |n| {
        inserted += n;
        on_progress(inserted, total_docs);
    })?;
    bulk_insert(&client, EDGE_COLLECTION, edge_docs, &mut |n| {
        inserted += n;
        on_progress(inserted, total_docs);
    })?;

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
    on_chunk: &mut dyn FnMut(usize),
) -> Result<(), String> {
    let query = format!("FOR d IN @docs INSERT d INTO {}", collection);
    for chunk in docs.chunks(INSERT_CHUNK) {
        let mut bind = HashMap::new();
        bind.insert("docs".to_string(), serde_json::Value::Array(chunk.to_vec()));
        client
            .query(&query, Some(bind))
            .map_err(|e| format!("bulk insert into {}: {}", collection, e))?;
        on_chunk(chunk.len());
    }
    Ok(())
}

/// Summary of a dev-server auto-reindex.
pub struct ReindexReport {
    pub nodes: usize,
    pub edges: usize,
    /// Nodes whose embedding was reused unchanged from SolidB.
    pub reused: usize,
    /// Nodes re-embedded because their text changed (or they're new).
    pub reembedded: usize,
}

/// Rebuild the graph and sync it to SolidB, reusing existing embeddings for
/// unchanged nodes so only what actually changed is re-embedded. `routes` is
/// the live server's route table (the dev reindex must not re-execute
/// `routes.sl`). Used by `soli serve --dev` when `SOLI_GRAPH_WATCH` is set.
pub fn reindex(
    app_path: &Path,
    database: Option<&str>,
    routes: &RouteSnapshot,
) -> Result<ReindexReport, String> {
    let mut graph =
        crate::graph::builder::build_graph_with_routes(app_path, routes, &mut |_, _| {})?;

    // Reuse cached embeddings for unchanged node text; embed only the deltas
    // (best-effort — a missing key keeps the baselines rather than erroring).
    let cache = fetch_embedding_cache(database);
    let (reused, reembedded) = attach_embeddings(&mut graph, &cache, true, false, &mut |_, _| {})?;

    let embed = graph.nodes.iter().any(|n| !n.embedding.is_empty());
    let opts = SyncOptions {
        database: database.map(str::to_string),
        embed,
    };
    // Non-destructive: upsert changed, delete removed — never drop the graph.
    let report = sync_graph(&graph, &opts, &mut |_, _| {})?;
    Ok(ReindexReport {
        nodes: report.nodes,
        edges: report.edges,
        reused,
        reembedded,
    })
}

/// Snapshot `{_key: (text, embedding)}` from the current node collection so a
/// rebuild can reuse vectors for nodes whose text is unchanged. Returns an
/// empty map if the collection is missing or unreachable (first build).
fn fetch_embedding_cache(database: Option<&str>) -> HashMap<String, (String, Vec<f64>)> {
    let Ok((client, _db)) = connect(database) else {
        return HashMap::new();
    };
    let query = "FOR n IN soli_graph_nodes RETURN { k: n._key, t: n.text, e: n.embedding }";
    let Ok(rows) = client.query(query, None) else {
        return HashMap::new();
    };
    let mut cache = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(key) = row.get("k").and_then(|v| v.as_str()) else {
            continue;
        };
        let text = row
            .get("t")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let vector: Vec<f64> = row
            .get("e")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_f64()).collect())
            .unwrap_or_default();
        cache.insert(key.to_string(), (text, vector));
    }
    cache
}

/// Attach embeddings incrementally: reuse a node's cached vector when its text
/// is unchanged; embed the rest (changed/new). `embed_new` gates whether misses
/// are embedded at all (`--no-embed` keeps only cached vectors). When `strict`,
/// a needed-but-unavailable embedding is a hard error (the CLI build); otherwise
/// the cached baseline is kept and re-embedding is best-effort (the dev
/// reindex, which must never leave semantic search dark on a save). Returns
/// `(reused, reembedded)`.
fn attach_embeddings(
    graph: &mut ProjectGraph,
    cache: &HashMap<String, (String, Vec<f64>)>,
    embed_new: bool,
    strict: bool,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<(usize, usize), String> {
    const NO_KEY: &str = "Embedding failed. Set SOLI_EMBEDDING_API_KEY (and \
        SOLI_EMBEDDING_URL / SOLI_EMBEDDING_MODEL for non-OpenAI providers), or \
        re-run with --no-embed.";

    let mut reused = 0usize;
    let mut miss_idx: Vec<usize> = Vec::new();
    for (index, node) in graph.nodes.iter_mut().enumerate() {
        if let Some((old_text, old_vec)) = cache.get(&node.key) {
            if !old_vec.is_empty() {
                // Reattach as a baseline so a changed node never loses its
                // vector even if re-embedding fails below.
                node.embedding = old_vec.clone();
                if old_text == &node.text {
                    reused += 1;
                    continue;
                }
            }
        }
        miss_idx.push(index);
    }

    let mut reembedded = 0usize;
    if embed_new && !miss_idx.is_empty() {
        if std::env::var("SOLI_EMBEDDING_API_KEY").is_err() {
            if strict {
                return Err(NO_KEY.to_string());
            }
            return Ok((reused, 0)); // reindex without a key: keep baselines
        }
        let texts: Vec<String> = miss_idx
            .iter()
            .map(|&i| graph.nodes[i].text.clone())
            .collect();
        let total = texts.len();
        let mut vectors: Vec<Vec<f64>> = Vec::with_capacity(total);
        for chunk in texts.chunks(EMBED_CHUNK) {
            match crate::embedding::generate_embeddings_batch(chunk) {
                Some(part) => vectors.extend(part),
                None => {
                    if strict {
                        return Err(NO_KEY.to_string());
                    }
                    break; // best-effort: keep baselines
                }
            }
            on_progress(vectors.len(), total);
        }
        for (offset, &index) in miss_idx.iter().enumerate() {
            if let Some(vector) = vectors.get(offset) {
                graph.nodes[index].embedding = vector.clone();
                reembedded += 1;
            }
        }
    }
    Ok((reused, reembedded))
}

/// Incremental embedding for the CLI build: reuse cached vectors for unchanged
/// nodes and embed the changed/new ones (strict — errors if embedding is needed
/// but unavailable). With `opts.embed == false` (`--no-embed`) it keeps the
/// existing vectors and embeds nothing. Returns `(reused, reembedded)`.
pub fn embed_incremental(
    graph: &mut ProjectGraph,
    opts: &SyncOptions,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<(usize, usize), String> {
    let cache = fetch_embedding_cache(opts.database.as_deref());
    attach_embeddings(graph, &cache, opts.embed, true, on_progress)
}

/// Non-destructive sync: upsert changed nodes/edges, delete the ones no longer
/// present, and store the file manifest — **without dropping** the collections.
/// A concurrent reader never sees an empty graph, and unchanged embeddings are
/// preserved. Deterministic edge `_key`s make the edge diff possible.
pub fn sync_graph(
    graph: &ProjectGraph,
    opts: &SyncOptions,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<SyncReport, String> {
    let (client, database) = connect(opts.database.as_deref())?;

    // Create-if-missing (never drop). Errors (e.g. "already exists") are fine.
    let _ = client.create_collection(NODE_COLLECTION, None);
    let _ = client.create_collection(EDGE_COLLECTION, Some("edge"));
    let _ = client.create_collection(META_COLLECTION, None);
    ensure_indexes(&client);

    let dimension = graph
        .nodes
        .iter()
        .map(|n| n.embedding.len())
        .find(|&d| d > 0)
        .unwrap_or(0);
    if opts.embed && dimension > 0 {
        // No-op if the vector index already exists.
        let _ = client.create_vector_index(
            NODE_COLLECTION,
            VECTOR_INDEX,
            "embedding",
            dimension,
            "cosine",
            None,
        );
    }

    let total = graph.nodes.len() + graph.edges.len();
    let mut done = 0usize;

    // Nodes: INSERT new, UPDATE changed (nodes keep their key while content
    // changes), REMOVE gone. `update_existing = true`.
    let node_docs: Vec<serde_json::Value> = graph
        .nodes
        .iter()
        .map(ProjectGraph::node_document)
        .collect();
    let new_node_keys: std::collections::HashSet<String> =
        graph.nodes.iter().map(|n| n.key.clone()).collect();
    let existing_nodes: std::collections::HashSet<String> =
        fetch_keys(&client, NODE_COLLECTION)?.into_iter().collect();
    upsert_docs(
        &client,
        NODE_COLLECTION,
        node_docs,
        &existing_nodes,
        true,
        &mut |n| {
            done += n;
            on_progress(done, total);
        },
    )?;
    let stale_nodes: Vec<String> = existing_nodes
        .iter()
        .filter(|k| !new_node_keys.contains(*k))
        .cloned()
        .collect();
    remove_keys(&client, NODE_COLLECTION, &stale_nodes)?;

    // Edges: their key hashes all their content, so a key present in both is
    // byte-identical — only INSERT new and REMOVE gone (`update_existing =
    // false`).
    let edge_docs: Vec<serde_json::Value> = graph
        .edges
        .iter()
        .map(ProjectGraph::edge_document)
        .collect();
    let new_edge_keys: std::collections::HashSet<String> = graph
        .edges
        .iter()
        .map(crate::graph::model::edge_key)
        .collect();
    let existing_edges: std::collections::HashSet<String> =
        fetch_keys(&client, EDGE_COLLECTION)?.into_iter().collect();
    upsert_docs(
        &client,
        EDGE_COLLECTION,
        edge_docs,
        &existing_edges,
        false,
        &mut |n| {
            done += n;
            on_progress(done, total);
        },
    )?;
    let stale_edges: Vec<String> = existing_edges
        .iter()
        .filter(|k| !new_edge_keys.contains(*k))
        .cloned()
        .collect();
    remove_keys(&client, EDGE_COLLECTION, &stale_edges)?;

    store_manifest(&client, &graph.file_hashes)?;

    Ok(SyncReport {
        nodes: graph.nodes.len(),
        edges: graph.edges.len(),
        database,
        embedded: opts.embed && dimension > 0,
        dimension: if opts.embed { dimension } else { 0 },
    })
}

/// True when the stored manifest matches the current file hashes and the node
/// collection is populated — i.e. a rebuild would be a no-op.
pub fn is_up_to_date(file_hashes: &HashMap<String, String>, database: Option<&str>) -> bool {
    let Ok((client, _db)) = connect(database) else {
        return false;
    };
    let stored = fetch_manifest(&client);
    if stored.is_empty() || &stored != file_hashes {
        return false;
    }
    fetch_keys(&client, NODE_COLLECTION)
        .map(|keys| !keys.is_empty())
        .unwrap_or(false)
}

fn ensure_indexes(client: &SoliDBClient) {
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
}

/// Upsert documents by `_key`, partitioning against the collection's existing
/// keys: new keys are `INSERT`ed and (when `update_existing`) present keys are
/// `UPDATE`d in place. This SolidB build supports `INSERT`/`UPDATE`/`REMOVE`
/// but not `UPSERT`/`REPLACE`, so we do the partition ourselves. Documents
/// carry all overwrite-sensitive fields (e.g. `superclass`, `doc`) so the
/// UPDATE fully replaces them.
fn upsert_docs(
    client: &SoliDBClient,
    collection: &str,
    docs: Vec<serde_json::Value>,
    existing: &std::collections::HashSet<String>,
    update_existing: bool,
    on_chunk: &mut dyn FnMut(usize),
) -> Result<(), String> {
    let mut inserts: Vec<serde_json::Value> = Vec::new();
    let mut updates: Vec<serde_json::Value> = Vec::new();
    for doc in docs {
        let is_existing = doc
            .get("_key")
            .and_then(|k| k.as_str())
            .map(|k| existing.contains(k))
            .unwrap_or(false);
        if is_existing {
            if update_existing {
                updates.push(doc);
            }
            // else: identical by key (edges) — leave untouched.
        } else {
            inserts.push(doc);
        }
    }
    run_bulk(
        client,
        &format!("FOR d IN @docs INSERT d INTO {}", collection),
        inserts,
        collection,
        on_chunk,
    )?;
    if update_existing {
        run_bulk(
            client,
            &format!("FOR d IN @docs UPDATE d._key WITH d IN {}", collection),
            updates,
            collection,
            on_chunk,
        )?;
    }
    Ok(())
}

/// Run a chunked bulk-mutation AQL over `docs` bound as `@docs`.
fn run_bulk(
    client: &SoliDBClient,
    query: &str,
    docs: Vec<serde_json::Value>,
    collection: &str,
    on_chunk: &mut dyn FnMut(usize),
) -> Result<(), String> {
    for chunk in docs.chunks(INSERT_CHUNK) {
        let mut bind = HashMap::new();
        bind.insert("docs".to_string(), serde_json::Value::Array(chunk.to_vec()));
        client
            .query(query, Some(bind))
            .map_err(|e| format!("bulk write into {}: {}", collection, e))?;
        on_chunk(chunk.len());
    }
    Ok(())
}

fn fetch_keys(client: &SoliDBClient, collection: &str) -> Result<Vec<String>, String> {
    let query = format!("FOR n IN {} RETURN n._key", collection);
    let rows = client
        .query(&query, None)
        .map_err(|e| format!("list keys of {}: {}", collection, e))?;
    Ok(rows
        .iter()
        .filter_map(|r| r.as_str().map(String::from))
        .collect())
}

fn remove_keys(client: &SoliDBClient, collection: &str, keys: &[String]) -> Result<(), String> {
    if keys.is_empty() {
        return Ok(());
    }
    let query = format!("FOR k IN @keys REMOVE k IN {}", collection);
    for chunk in keys.chunks(INSERT_CHUNK) {
        let mut bind = HashMap::new();
        bind.insert("keys".to_string(), serde_json::json!(chunk));
        client
            .query(&query, Some(bind))
            .map_err(|e| format!("remove from {}: {}", collection, e))?;
    }
    Ok(())
}

fn fetch_manifest(client: &SoliDBClient) -> HashMap<String, String> {
    match client.get(META_COLLECTION, "manifest") {
        Ok(Some(doc)) => doc
            .get("files")
            .and_then(|f| f.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        _ => HashMap::new(),
    }
}

fn store_manifest(client: &SoliDBClient, files: &HashMap<String, String>) -> Result<(), String> {
    let doc = serde_json::json!({ "_key": "manifest", "files": files });
    let exists = client
        .get(META_COLLECTION, "manifest")
        .map(|d| d.is_some())
        .unwrap_or(false);
    let query = if exists {
        format!("FOR d IN @docs UPDATE d._key WITH d IN {}", META_COLLECTION)
    } else {
        format!("FOR d IN @docs INSERT d INTO {}", META_COLLECTION)
    };
    let mut bind = HashMap::new();
    bind.insert("docs".to_string(), serde_json::json!([doc]));
    client
        .query(&query, Some(bind))
        .map_err(|e| format!("store manifest: {}", e))?;
    Ok(())
}

/// Build a SolidB client from the model-layer config, honoring an optional
/// database override. Auth priority mirrors `crud.rs`: JWT > API key > basic.
/// Shared by the write path and the query path.
pub(crate) fn connect(database: Option<&str>) -> Result<(SoliDBClient, String), String> {
    let host = format!(
        "{}{}",
        db_config::DB_CONFIG.scheme,
        db_config::DB_CONFIG.host
    );
    let database = database
        .map(str::to_string)
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
