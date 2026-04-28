use reqwest;
use serde_json::Value;
use std::collections::HashMap;

use crate::interpreter::builtins::http_class::get_http_client;

fn deserialize_msgpack(bytes: &[u8]) -> Result<Value, SoliDBError> {
    rmp_serde::from_slice(bytes).map_err(|e| SoliDBError {
        message: format!("MessagePack deserialization error: {}", e),
        code: None,
    })
}
use crate::serve::get_tokio_handle;

// Fallback tokio runtime for SoliDB operations outside of a server context
// (e.g., migrations, REPL). Uses a lightweight current-thread runtime.
thread_local! {
    static FALLBACK_RT: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create fallback tokio runtime");
}

/// Run an async future synchronously, using the server's tokio handle if available,
/// otherwise falling back to a lightweight per-thread runtime.
///
/// If called from within an async runtime context, creates a dedicated single-thread
/// runtime to avoid blocking the I/O driver and causing potential deadlocks.
fn block_on<F>(future: F) -> F::Output
where
    F: std::future::Future + 'static,
{
    if let Some(rt) = get_tokio_handle() {
        if tokio::runtime::Handle::try_current().is_ok() {
            // Already inside async runtime — create a dedicated single-thread runtime
            // so we don't block the caller's I/O driver thread
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(future)
        } else {
            // Outside async context — safe to block on the runtime handle
            rt.block_on(future)
        }
    } else {
        FALLBACK_RT.with(|rt| rt.block_on(future))
    }
}

pub struct SoliDBClient {
    base_url: String,
    database: Option<String>,
    api_key: Option<String>,
    jwt_token: Option<String>,
    username: Option<String>,
    password: Option<String>,
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
            jwt_token: None,
            username: None,
            password: None,
        })
    }

    pub fn with_api_key(mut self, api_key: &str) -> Self {
        self.api_key = Some(api_key.to_string());
        self
    }

    pub fn with_jwt_token(mut self, token: &str) -> Self {
        self.jwt_token = Some(token.to_string());
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
        let client = get_http_client().clone();
        let mut request = client.request(method.clone(), &url);

        // Auth priority: JWT (fastest) > API key > Basic auth
        if let Some(jwt) = &self.jwt_token {
            request = request.header("Authorization", format!("Bearer {}", jwt));
        } else if let Some(api_key) = &self.api_key {
            request = request.header("x-api-key", api_key);
        } else if let (Some(u), Some(p)) = (&self.username, &self.password) {
            request = request.basic_auth(u, Some(p));
        }

        request = request.header("Accept", "application/json");

        if let Some(b) = body {
            let json_bytes = serde_json::to_vec(b).map_err(|e| SoliDBError {
                message: format!("Failed to serialize request body: {}", e),
                code: None,
            })?;
            request = request
                .header("Content-Type", "application/json")
                .body(json_bytes);
        }

        let path_owned = path.to_string();
        let method_clone = method.clone();

        // Single block_on for the entire operation (send + read response).
        // Holding a response body stream outside a tokio context causes
        // "there is no reactor running" panics with reqwest 0.12.
        block_on(async move {
            let response = request.send().await.map_err(|e| SoliDBError {
                message: format!("HTTP request failed: {}", e),
                code: None,
            })?;

            let status = response.status();

            if !status.is_success() {
                let error_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                return Err(SoliDBError {
                    message: format!("HTTP {} {}: {}", status, path_owned, error_text),
                    code: Some(status.as_u16() as i32),
                });
            }

            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let bytes = response.bytes().await.map_err(|e| SoliDBError {
                message: format!("Failed to read response: {}", e),
                code: None,
            })?;

            if bytes.is_empty() {
                return Err(SoliDBError {
                    message: format!("Empty response for HTTP {} {}", method_clone, path_owned),
                    code: None,
                });
            }

            if content_type.contains("msgpack") {
                deserialize_msgpack(&bytes)
            } else {
                // JSON fallback — use serde_json to parse into serde_json::Value
                // (needed because SoliDB client returns serde_json::Value, not our Value)
                serde_json::from_slice(&bytes).map_err(|e| SoliDBError {
                    message: format!(
                        "Failed to parse response: {} - Body: {}",
                        e,
                        String::from_utf8_lossy(&bytes)
                    ),
                    code: None,
                })
            }
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

    pub fn create_collection(
        &self,
        name: &str,
        collection_type: Option<&str>,
    ) -> Result<(), SoliDBError> {
        let db = self.get_db()?;
        let mut body = serde_json::json!({"name": name});
        if let Some(ct) = collection_type {
            body["type"] = serde_json::Value::String(ct.to_string());
        }
        self.request(
            reqwest::Method::POST,
            &format!("/_api/database/{}/collection", db),
            Some(&body),
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
        _merge: bool,
    ) -> Result<Value, SoliDBError> {
        let db = self.get_db()?;
        let path = format!("/_api/database/{}/document/{}/{}", db, collection, key);
        let response: Value = self.request(reqwest::Method::PUT, &path, Some(&document))?;
        Ok(response)
    }

    pub fn delete(&self, collection: &str, key: &str) -> Result<(), SoliDBError> {
        let db = self.get_db()?;
        let path = format!("/_api/database/{}/document/{}/{}", db, collection, key);
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

    /// Upload a blob using SoliDB's native blob API
    /// (`POST /_api/blob/{db}/{collection}`). The endpoint chunks the file and
    /// stores the chunks in the blob collection, so downloads via either the
    /// blob HTTP API or the DB admin UI see real binary data rather than a
    /// document with a base64 `data` field. Previously this method posted a
    /// regular document with the base64-encoded body, which meant blobs
    /// written from Soli were unreadable from anything that used the real
    /// blob endpoints.
    pub fn store_blob(
        &self,
        collection: &str,
        data: &[u8],
        filename: &str,
        content_type: &str,
    ) -> Result<String, SoliDBError> {
        let db = self.get_db()?.to_string();
        let url = format!("{}/_api/blob/{}/{}", self.base_url, db, collection);
        let client = get_http_client().clone();

        let data_owned = data.to_vec();
        let filename = filename.to_string();
        let content_type = content_type.to_string();

        let jwt = self.jwt_token.clone();
        let api_key = self.api_key.clone();
        let basic = self.username.clone().zip(self.password.clone());

        block_on(async move {
            let part = reqwest::multipart::Part::bytes(data_owned)
                .file_name(filename)
                .mime_str(&content_type)
                .map_err(|e| SoliDBError {
                    message: format!("Invalid content type: {}", e),
                    code: None,
                })?;
            let form = reqwest::multipart::Form::new().part("file", part);

            let mut request = client.post(&url).multipart(form);
            if let Some(jwt) = jwt {
                request = request.header("Authorization", format!("Bearer {}", jwt));
            } else if let Some(api_key) = api_key {
                request = request.header("x-api-key", api_key);
            } else if let Some((u, p)) = basic {
                request = request.basic_auth(u, Some(p));
            }
            request = request.header("Accept", "application/json");

            let response = request.send().await.map_err(|e| SoliDBError {
                message: format!("HTTP request failed: {}", e),
                code: None,
            })?;

            let status = response.status();
            if !status.is_success() {
                let error_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                return Err(SoliDBError {
                    message: format!("HTTP {} {}: {}", status, url, error_text),
                    code: Some(status.as_u16() as i32),
                });
            }

            let doc: Value = response.json().await.map_err(|e| SoliDBError {
                message: format!("Failed to parse upload response: {}", e),
                code: None,
            })?;

            doc.get("_key")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| SoliDBError {
                    message: "Blob upload response missing _key".to_string(),
                    code: None,
                })
        })
    }

    /// Download a blob's raw bytes via SoliDB's native blob API
    /// (`GET /_api/blob/{db}/{collection}/{key}`). Returns the unencoded bytes
    /// the browser (or any client) can use directly.
    pub fn get_blob(&self, collection: &str, blob_id: &str) -> Result<Vec<u8>, SoliDBError> {
        let db = self.get_db()?.to_string();
        let blob_id = blob_id.to_string();
        let url = format!(
            "{}/_api/blob/{}/{}/{}",
            self.base_url, db, collection, blob_id
        );
        let client = get_http_client().clone();

        let jwt = self.jwt_token.clone();
        let api_key = self.api_key.clone();
        let basic = self.username.clone().zip(self.password.clone());

        block_on(async move {
            let mut request = client.get(&url);
            if let Some(jwt) = jwt {
                request = request.header("Authorization", format!("Bearer {}", jwt));
            } else if let Some(api_key) = api_key {
                request = request.header("x-api-key", api_key);
            } else if let Some((u, p)) = basic {
                request = request.basic_auth(u, Some(p));
            }

            let response = request.send().await.map_err(|e| SoliDBError {
                message: format!("HTTP request failed: {}", e),
                code: None,
            })?;

            let status = response.status();
            if !status.is_success() {
                let error_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                return Err(SoliDBError {
                    message: format!("HTTP {} {}: {}", status, url, error_text),
                    code: Some(status.as_u16() as i32),
                });
            }

            let bytes = response.bytes().await.map_err(|e| SoliDBError {
                message: format!("Failed to read blob bytes: {}", e),
                code: None,
            })?;
            Ok(bytes.to_vec())
        })
    }

    /// Fetch blob metadata (the document that describes a blob — name, type,
    /// size, chunks, created). Also exposes `filename` and `content_type`
    /// aliases for callers written against the previous document-backed
    /// implementation of this client.
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
        // Compat aliases: the native blob doc uses `name`/`type`, but existing
        // callers read `filename`/`content_type`. Expose both so we don't
        // break them.
        if !metadata.contains_key("filename") {
            if let Some(name) = metadata.get("name").cloned() {
                metadata.insert("filename".to_string(), name);
            }
        }
        if !metadata.contains_key("content_type") {
            if let Some(ty) = metadata.get("type").cloned() {
                metadata.insert("content_type".to_string(), ty);
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

    // ===== Queue API =====

    /// List all queues in the current database.
    pub fn list_queues(&self) -> Result<Vec<Value>, SoliDBError> {
        let db = self.get_db()?;
        let response: Value = self.request(
            reqwest::Method::GET,
            &format!("/_api/database/{}/queues", db),
            None,
        )?;
        Ok(extract_array(&response, &["queues", "result", "data"]))
    }

    /// List jobs in a queue.
    pub fn list_jobs(&self, queue: &str) -> Result<Vec<Value>, SoliDBError> {
        let db = self.get_db()?;
        let response: Value = self.request(
            reqwest::Method::GET,
            &format!("/_api/database/{}/queues/{}/jobs", db, queue),
            None,
        )?;
        Ok(extract_array(&response, &["jobs", "result", "data"]))
    }

    /// Enqueue a job. `run_at` is an optional ISO-8601 timestamp for delayed
    /// execution; when `None`, the job is run as soon as a worker picks it up.
    pub fn enqueue_job(
        &self,
        queue: &str,
        handler: &str,
        args: Value,
        callback_url: &str,
        run_at: Option<&str>,
    ) -> Result<String, SoliDBError> {
        let db = self.get_db()?;
        let mut payload = serde_json::json!({
            "handler": handler,
            "args": args,
            "callback_url": callback_url,
        });
        if let Some(when) = run_at {
            payload["run_at"] = serde_json::Value::String(when.to_string());
        }
        let response: Value = self.request(
            reqwest::Method::POST,
            &format!("/_api/database/{}/queues/{}/enqueue", db, queue),
            Some(&payload),
        )?;
        Ok(extract_id(&response))
    }

    /// Cancel an enqueued (not yet started) job by id.
    pub fn cancel_job(&self, job_id: &str) -> Result<(), SoliDBError> {
        let db = self.get_db()?;
        self.request(
            reqwest::Method::DELETE,
            &format!("/_api/database/{}/queues/jobs/{}", db, job_id),
            None,
        )?;
        Ok(())
    }

    // ===== Cron API =====

    /// List all cron entries in the current database.
    pub fn list_crons(&self) -> Result<Vec<Value>, SoliDBError> {
        let db = self.get_db()?;
        let response: Value = self.request(
            reqwest::Method::GET,
            &format!("/_api/database/{}/cron", db),
            None,
        )?;
        Ok(extract_array(&response, &["crons", "result", "data"]))
    }

    /// Create a new cron entry. Returns the SolidB-issued id.
    pub fn create_cron(
        &self,
        name: &str,
        expr: &str,
        handler: &str,
        args: Value,
        callback_url: &str,
    ) -> Result<String, SoliDBError> {
        let db = self.get_db()?;
        let payload = serde_json::json!({
            "name": name,
            "schedule": expr,
            "handler": handler,
            "args": args,
            "callback_url": callback_url,
        });
        let response: Value = self.request(
            reqwest::Method::POST,
            &format!("/_api/database/{}/cron", db),
            Some(&payload),
        )?;
        Ok(extract_id(&response))
    }

    /// Update a cron entry. `fields` is a JSON object of fields to change.
    pub fn update_cron(&self, id: &str, fields: Value) -> Result<(), SoliDBError> {
        let db = self.get_db()?;
        self.request(
            reqwest::Method::PUT,
            &format!("/_api/database/{}/cron/{}", db, id),
            Some(&fields),
        )?;
        Ok(())
    }

    /// Delete a cron entry by id.
    pub fn delete_cron(&self, id: &str) -> Result<(), SoliDBError> {
        let db = self.get_db()?;
        self.request(
            reqwest::Method::DELETE,
            &format!("/_api/database/{}/cron/{}", db, id),
            None,
        )?;
        Ok(())
    }
}

/// Pull an array out of a SolidB list-style response, trying common envelope
/// keys before falling back to treating the response itself as the array.
fn extract_array(response: &Value, keys: &[&str]) -> Vec<Value> {
    for key in keys {
        if let Some(arr) = response.get(*key).and_then(|v| v.as_array()) {
            return arr.clone();
        }
    }
    response.as_array().cloned().unwrap_or_default()
}

/// Pull an id out of a create-style response. SolidB endpoints have used
/// `id`, `_key`, and `job_id` historically; accept any.
fn extract_id(response: &Value) -> String {
    for key in ["id", "_key", "job_id", "cron_id"] {
        if let Some(s) = response.get(key).and_then(|v| v.as_str()) {
            return s.to_string();
        }
    }
    // Some endpoints may return a bare string id.
    if let Some(s) = response.as_str() {
        return s.to_string();
    }
    String::new()
}
