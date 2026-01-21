//! ModelResult - Lazy query iterator for Model operations.

use crate::interpreter::value::Value;

#[derive(Debug, Clone)]
pub struct Filter {
    pub field: String,
    pub operator: String,
    pub value: Value,
}

impl Filter {
    pub fn new(field: &str, operator: &str, value: Value) -> Self {
        Self {
            field: field.to_string(),
            operator: operator.to_string(),
            value,
        }
    }
}

#[derive(Debug, Default)]
pub struct QueryState {
    pub filters: Vec<Filter>,
    pub sort_field: Option<String>,
    pub sort_order: Option<String>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
    pub model_name: String,
    pub collection: String,
    pub database: String,
}

impl QueryState {
    pub fn new(model_name: &str, collection: &str, database: &str) -> Self {
        Self {
            model_name: model_name.to_string(),
            collection: collection.to_string(),
            database: database.to_string(),
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub struct ModelResult {
    state: QueryState,
}

impl ModelResult {
    pub fn new(model_name: &str, collection: &str, database: &str) -> Self {
        Self {
            state: QueryState::new(model_name, collection, database),
        }
    }

    pub fn where_clause(&mut self, field: &str, operator: &str, value: Value) -> &mut Self {
        self.state.filters.push(Filter::new(field, operator, value));
        self
    }

    pub fn and_where(&mut self, field: &str, operator: &str, value: Value) -> &mut Self {
        self.where_clause(field, operator, value)
    }

    pub fn order_by(&mut self, field: &str, order: &str) -> &mut Self {
        self.state.sort_field = Some(field.to_string());
        self.state.sort_order = Some(order.to_string());
        self
    }

    pub fn offset(&mut self, n: u64) -> &mut Self {
        self.state.offset = Some(n);
        self
    }

    pub fn limit(&mut self, n: u64) -> &mut Self {
        self.state.limit = Some(n);
        self
    }

    pub fn count(&self) -> Result<u64, String> {
        Ok(0)
    }

    pub fn exists(&self) -> Result<bool, String> {
        Ok(false)
    }

    pub fn first(&mut self) -> Result<Option<Value>, String> {
        Ok(None)
    }

    pub fn all(&self) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }

    pub fn to_sdbql(&self) -> String {
        let mut query = format!("FOR d IN {}", self.state.collection);

        if !self.state.filters.is_empty() {
            query.push_str(" FILTER ");
            let filters: Vec<String> = self
                .state
                .filters
                .iter()
                .map(|f| format!("d.{} {} @p_{}", f.field, f.operator, f.field))
                .collect();
            query.push_str(&filters.join(" AND "));
        }

        if let Some(ref field) = self.state.sort_field {
            let order = self.state.sort_order.as_deref().unwrap_or("ASC");
            query.push_str(&format!(" SORT d.{} {}", field, order.to_uppercase()));
        }

        if let Some(offset) = self.state.offset {
            query.push_str(&format!(" OFFSET {}", offset));
        }

        if let Some(limit) = self.state.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        query.push_str(" RETURN d");
        query
    }

    pub fn paginate(&mut self, page: u64, per_page: u64) -> &mut Self {
        self.state.offset = Some((page - 1) * per_page);
        self.state.limit = Some(per_page);
        self
    }
}
