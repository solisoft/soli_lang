//! `soli graph` — build a graph of a Soli project and store it in SolidB so
//! agents can retrieve code by semantic search *and* traverse relationships
//! (graph RAG).
//!
//! - [`builder::build_graph`] walks the app's source and produces a
//!   [`model::ProjectGraph`] (nodes + edges) with no I/O.
//! - [`sync::embed_graph`] embeds every node's text.
//! - [`sync::write_graph`] drops + recreates the `soli_graph_nodes` /
//!   `soli_graph_edges` collections in SolidB and bulk-inserts the graph.
//!
//! The CLI handler (`cli::commands::run_graph`) orchestrates these; the module
//! itself stays free of terminal/formatting concerns so it is unit-testable.

pub mod builder;
pub mod model;
pub mod query;
pub mod sync;

pub use builder::build_graph;
pub use model::{Edge, Node, ProjectGraph};
pub use query::{run_query, QueryOptions, QueryResult};
pub use sync::{embed_graph, write_graph, SyncOptions, SyncReport};
