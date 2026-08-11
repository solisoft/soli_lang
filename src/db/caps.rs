//! What each backend can do. Used for capability checks and docs matrix.

/// Feature flags for the selected database backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendCaps {
    pub implemented: bool,
    pub hash_where: bool,
    pub string_sdbql_where: bool,
    pub associations: bool,
    pub aggregates: bool,
    pub transactions: bool,
    /// Multi-read coalesce (`grouped`) in one network round-trip.
    pub grouped_coalesce: bool,
    pub graph: bool,
    pub vector_search: bool,
    pub fulltext: bool,
    pub geo: bool,
    pub columnar: bool,
    pub timeseries: bool,
    pub live_queries: bool,
    /// Auto-create collection/table on first write.
    pub auto_create_collection: bool,
    pub raw_sdbql: bool,
    pub raw_sql: bool,
}

impl BackendCaps {
    pub const fn solidb() -> Self {
        Self {
            implemented: true,
            hash_where: true,
            string_sdbql_where: true,
            associations: true,
            aggregates: true,
            transactions: true,
            grouped_coalesce: true,
            graph: true,
            vector_search: true,
            fulltext: true,
            geo: true,
            columnar: true,
            timeseries: true,
            live_queries: true,
            auto_create_collection: true,
            raw_sdbql: true,
            raw_sql: false,
        }
    }

    /// PostgreSQL document backend (CRUD, hash where, aggregates, migrations).
    pub const fn postgres() -> Self {
        Self {
            implemented: true,
            hash_where: true,
            string_sdbql_where: false,
            associations: true, // includes batching (belongs_to/has_many/has_one)
            aggregates: true,   // sum/avg/min/max/count + multi-row group_by
            transactions: true, // Model.transaction holds one pool connection
            grouped_coalesce: false,
            graph: false,
            vector_search: false,
            fulltext: false,
            geo: false,
            columnar: false,
            timeseries: false,
            live_queries: false,
            // Document tables are created on first write (ensure_table).
            auto_create_collection: true,
            raw_sdbql: false,
            raw_sql: true,
        }
    }

    /// MySQL document backend — same capability surface as Postgres.
    pub const fn mysql() -> Self {
        Self::postgres()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solidb_is_full_featured() {
        let c = BackendCaps::solidb();
        assert!(c.implemented);
        assert!(c.graph && c.vector_search && c.grouped_coalesce);
        assert!(c.string_sdbql_where && c.raw_sdbql);
        assert!(!c.raw_sql);
    }

    #[test]
    fn postgres_is_implemented_subset() {
        let c = BackendCaps::postgres();
        assert!(c.implemented);
        assert!(c.hash_where);
        assert!(c.associations); // Phase 3 includes batching
        assert!(c.aggregates);
        assert!(c.transactions);
        assert!(!c.string_sdbql_where && !c.graph);
        assert!(!c.vector_search); // pgvector still optional / deferred
        assert!(c.raw_sql);
    }

    #[test]
    fn mysql_matches_postgres_caps() {
        let c = BackendCaps::mysql();
        assert!(c.implemented && c.aggregates && c.associations && c.transactions);
        assert!(!c.string_sdbql_where);
    }
}
