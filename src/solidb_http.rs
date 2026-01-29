use reqwest;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

// Global shared HTTP client for connection pooling
static SHARED_CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

fn get_shared_client() -> &'static reqwest::blocking::Client {
    SHARED_CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(100)
            .tcp_keepalive(Duration::from_secs(60))
            .build()
            .expect("Failed to create HTTP client")
    })
}

pub struct SoliDBClient {
    base_url: String,
    database: Option<String>,
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
    client: &'static reqwest::blocking::Client,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct SoliDBError {
    message: String,
    code: Option<i32>,
}

impl std::fmt::Display for SoliDBError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SoliDBError {}

impl From<reqwest::Error> for SoliDBError {
    fn from(e: reqwest::Error) -> Self {
        SoliDBError {
            message: format!("HTTP error: {}", e),
            code: None,
        }
    }
}

impl SoliDBClient {
    pub fn connect(host: &str) -> Result<Self, SoliDBError> {
        // Add http:// scheme if missing
        let base_url = if host.starts_with("http://") || host.starts_with("https://") {
            host.trim_end_matches('/').to_string()
        } else {
            format!("http://{}", host.trim_end_matches('/'))
        };

        Ok(Self {
            base_url,
            database: None,
            api_key: None,
            username: None,
            password: None,
            client: get_shared_client(),
        })
    }

    pub fn with_api_key(mut self, api_key: &str) -> Self {
        self.api_key = Some(api_key.to_string());
        self
    }

    pub fn with_basic_auth(mut self, username: &str, password: &str) -> Self {
        self.username = Some(username.to_string());
        self.password = Some(password.to_string());
        self
    }

    pub fn set_database(&mut self, database: &str) {
        self.database = Some(database.to_string());
    }

    fn get_db(&self) -> Result<&str, SoliDBError> {
        self.database.as_deref().ok_or_else(|| SoliDBError {
            message: "No database specified".to_string(),
            code: None,
        })
    }

    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, SoliDBError> {
        let url = format!("{}{}", self.base_url, path);
        let mut request = self.client.request(method.clone(), &url);

        if let Some(api_key) = &self.api_key {
            request = request.header("x-api-key", api_key);
        }

        if let (Some(u), Some(p)) = (&self.username, &self.password) {
            request = request.basic_auth(u, Some(p));
        }

        if let Some(b) = body {
            request = request.json(b);
        }

        let response = request.send().map_err(|e| SoliDBError {
            message: format!("HTTP request failed: {}", e),
            code: None,
        })?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response
                .text()
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(SoliDBError {
                message: format!("HTTP {} {}: {}", status, path, error_text),
                code: Some(status.as_u16() as i32),
            });
        }

        let text = response.text().map_err(|e| SoliDBError {
            message: format!("Failed to read response: {}", e),
            code: None,
        })?;

        if text.is_empty() {
            return Err(SoliDBError {
                message: format!("Empty response for HTTP {} {}", method, path),
                code: None,
            });
        }

        serde_json::from_str(&text).map_err(|e| SoliDBError {
            message: format!("Failed to parse JSON: {} - Text: {}", e, text),
            code: None,
        })
    }

    pub fn ping(&self) -> Result<bool, SoliDBError> {
        // Do a simple query to check connectivity
        let db = self.database.as_deref().unwrap_or("solidb");
        let path = format!("/_api/database/{}/cursor", db);
        let _ = self.request(
            reqwest::Method::POST,
            &path,
            Some(&serde_json::json!({
                "query": "RETURN 1"
            })),
        )?;
        Ok(true)
    }

    pub fn list_databases(&self) -> Result<Vec<String>, SoliDBError> {
        let response: Value = self.request(reqwest::Method::GET, "/_api/databases", None)?;
        Ok(response
            .get("databases")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn create_database(&self, name: &str) -> Result<(), SoliDBError> {
        self.request(
            reqwest::Method::POST,
            "/_api/databases",
            Some(&serde_json::json!({"name": name})),
        )?;
        Ok(())
    }

    pub fn delete_database(&self, name: &str) -> Result<(), SoliDBError> {
        self.request(
            reqwest::Method::DELETE,
            &format!("/_api/databases/{}", name),
            None,
        )?;
        Ok(())
    }

    pub fn list_collections(&self) -> Result<Vec<Value>, SoliDBError> {
        let db = self.get_db()?;
        let response: Value = self.request(
            reqwest::Method::GET,
            &format!("/_api/database/{}/collection", db),
            None,
        )?;
        Ok(response
            .get("collections")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub fn create_collection(&self, name: &str) -> Result<(), SoliDBError> {
        let db = self.get_db()?;
        self.request(
            reqwest::Method::POST,
            &format!("/_api/database/{}/collection", db),
            Some(&serde_json::json!({"name": name})),
        )?;
        Ok(())
    }

    pub fn drop_collection(&self, name: &str) -> Result<(), SoliDBError> {
        let db = self.get_db()?;
        self.request(
            reqwest::Method::DELETE,
            &format!("/_api/database/{}/collection/{}", db, name),
            None,
        )?;
        Ok(())
    }

    pub fn insert(
        &self,
        collection: &str,
        key: Option<&str>,
        mut document: Value,
    ) -> Result<Value, SoliDBError> {
        let db = self.get_db()?;
        if let Some(k) = key {
            if let Some(obj) = document.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::json!(k));
            }
        }
        let path = format!("/_api/database/{}/document/{}", db, collection);
        self.request(reqwest::Method::POST, &path, Some(&document))
    }

    pub fn get(&self, collection: &str, key: &str) -> Result<Option<Value>, SoliDBError> {
        let db = self.get_db()?;
        let path = format!("/_api/database/{}/document/{}/{}", db, collection, key);
        let response: Value = self.request(reqwest::Method::GET, &path, None)?;
        Ok(Some(response))
    }

    pub fn update(
        &self,
        collection: &str,
        key: &str,
        document: Value,
        merge: bool,
    ) -> Result<Value, SoliDBError> {
        let db = self.get_db()?;
        let payload = serde_json::json!({
            "document": document,
            "merge": merge
        });
        let path = format!("/_api/database/{}/document/{}/{}", db, collection, key);
        let response: Value = self.request(reqwest::Method::PUT, &path, Some(&payload))?;
        Ok(response)
    }

    pub fn delete(&self, collection: &str, key: &str) -> Result<(), SoliDBError> {
        let db = self.get_db()?;
        let path = format!(
            "/_api/database/{}/collection/{}/document/{}",
            db, collection, key
        );
        self.request(reqwest::Method::DELETE, &path, None)?;
        Ok(())
    }

    pub fn list(
        &self,
        collection: &str,
        limit: i32,
        offset: i32,
    ) -> Result<Vec<Value>, SoliDBError> {
        let db = self.get_db()?;
        let path = format!(
            "/_api/database/{}/collection/{}/documents?limit={}&offset={}",
            db, collection, limit, offset
        );
        let response: Value = self.request(reqwest::Method::GET, &path, None)?;
        Ok(response
            .get("documents")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub fn query(
        &self,
        sdbql: &str,
        bind_vars: Option<HashMap<String, Value>>,
    ) -> Result<Vec<Value>, SoliDBError> {
        let db = self.get_db()?;
        let mut payload = serde_json::json!({
            "query": sdbql
        });
        if let Some(bv) = bind_vars {
            payload["bindVars"] = serde_json::json!(bv);
        }
        let path = format!("/_api/database/{}/cursor", db);
        let response: Value = self.request(reqwest::Method::POST, &path, Some(&payload))?;
        Ok(response
            .get("result")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub fn explain(
        &self,
        sdbql: &str,
        bind_vars: Option<HashMap<String, Value>>,
    ) -> Result<Value, SoliDBError> {
        let db = self.get_db()?;
        let mut payload = serde_json::json!({
            "query": sdbql
        });
        if let Some(bv) = bind_vars {
            payload["bindVars"] = serde_json::json!(bv);
        }
        let path = format!("/_api/database/{}/explain", db);
        let response: Value = self.request(reqwest::Method::POST, &path, Some(&payload))?;
        Ok(response)
    }

    pub fn begin_transaction(&self, isolation_level: Option<&str>) -> Result<String, SoliDBError> {
        let db = self.get_db()?;
        let mut payload = serde_json::json!({
            "database": db
        });
        if let Some(il) = isolation_level {
            payload["isolation_level"] = serde_json::json!(il);
        }
        let response: Value = self.request(
            reqwest::Method::POST,
            "/_api/transaction/begin",
            Some(&payload),
        )?;
        response
            .get("tx_id")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| SoliDBError {
                message: "No tx_id in response".to_string(),
                code: None,
            })
    }

    pub fn commit_transaction(&self, tx_id: &str) -> Result<(), SoliDBError> {
        self.request(
            reqwest::Method::POST,
            "/_api/transaction/commit",
            Some(&serde_json::json!({"tx_id": tx_id})),
        )?;
        Ok(())
    }

    pub fn rollback_transaction(&self, tx_id: &str) -> Result<(), SoliDBError> {
        self.request(
            reqwest::Method::POST,
            "/_api/transaction/rollback",
            Some(&serde_json::json!({"tx_id": tx_id})),
        )?;
        Ok(())
    }

    pub fn create_index(
        &self,
        collection: &str,
        name: &str,
        fields: Vec<String>,
        unique: bool,
        sparse: bool,
    ) -> Result<Value, SoliDBError> {
        let db = self.get_db()?;
        let payload = serde_json::json!({
            "name": name,
            "type": "hash",
            "fields": fields,
            "unique": unique,
            "sparse": sparse
        });
        let path = format!("/_api/database/{}/{}/indexes", db, collection);
        let response: Value = self.request(reqwest::Method::POST, &path, Some(&payload))?;
        Ok(response)
    }

    pub fn drop_index(&self, collection: &str, name: &str) -> Result<(), SoliDBError> {
        let db = self.get_db()?;
        let path = format!("/_api/database/{}/{}/indexes/{}", db, collection, name);
        self.request(reqwest::Method::DELETE, &path, None)?;
        Ok(())
    }

    pub fn list_indexes(&self, collection: &str) -> Result<Vec<Value>, SoliDBError> {
        let db = self.get_db()?;
        let path = format!("/_api/database/{}/{}/indexes", db, collection);
        let response: Value = self.request(reqwest::Method::GET, &path, None)?;
        Ok(response
            .get("indexes")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub fn collection_stats(&self, collection: &str) -> Result<Value, SoliDBError> {
        let db = self.get_db()?;
        let path = format!("/_api/database/{}/collection/{}/stats", db, collection);
        let response: Value = self.request(reqwest::Method::GET, &path, None)?;
        Ok(response)
    }

    pub fn store_blob(
        &self,
        collection: &str,
        data: &[u8],
        filename: &str,
        content_type: &str,
    ) -> Result<String, SoliDBError> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        use uuid::Uuid;

        let db = self.get_db()?;
        let blob_id = Uuid::new_v4().to_string();
        let encoded = STANDARD.encode(data);

        let document = serde_json::json!({
            "_key": blob_id,
            "filename": filename,
            "content_type": content_type,
            "size": data.len(),
            "data": encoded,
            "created_at": chrono::Utc::now().to_rfc3339()
        });

        let path = format!("/_api/database/{}/document/{}", db, collection);
        self.request(reqwest::Method::POST, &path, Some(&document))?;

        Ok(blob_id)
    }

    pub fn get_blob(&self, collection: &str, blob_id: &str) -> Result<Vec<u8>, SoliDBError> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let db = self.get_db()?;
        let path = format!("/_api/database/{}/document/{}/{}", db, collection, blob_id);
        let response: Value = self.request(reqwest::Method::GET, &path, None)?;

        let data_str = response
            .get("data")
            .and_then(|d| d.as_str())
            .ok_or_else(|| SoliDBError {
                message: "Blob data not found".to_string(),
                code: None,
            })?;

        STANDARD.decode(data_str).map_err(|e| SoliDBError {
            message: format!("Failed to decode blob: {}", e),
            code: None,
        })
    }

    pub fn get_blob_metadata(
        &self,
        collection: &str,
        blob_id: &str,
    ) -> Result<serde_json::Value, SoliDBError> {
        let db = self.get_db()?;
        let path = format!("/_api/database/{}/document/{}/{}", db, collection, blob_id);
        let response: Value = self.request(reqwest::Method::GET, &path, None)?;

        let mut metadata = serde_json::Map::new();
        if let Some(obj) = response.as_object() {
            for (k, v) in obj.iter() {
                if k != "data" {
                    metadata.insert(k.clone(), v.clone());
                }
            }
        }

        Ok(serde_json::Value::Object(metadata))
    }

    pub fn delete_blob(&self, collection: &str, blob_id: &str) -> Result<(), SoliDBError> {
        let db = self.get_db()?;
        let path = format!("/_api/database/{}/document/{}/{}", db, collection, blob_id);
        self.request(reqwest::Method::DELETE, &path, None)?;
        Ok(())
    }
}
