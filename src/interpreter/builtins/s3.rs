//! S3 built-in class for SoliLang.
//!
//! Provides the S3 class with static methods for S3 operations:
//! - S3.list_buckets() -> Array
//! - S3.create_bucket(name) -> Bool
//! - S3.delete_bucket(name) -> Bool
//! - S3.put_object(bucket, key, body, options?) -> Bool
//! - S3.get_object(bucket, key) -> String (UTF-8 text content only)
//! - S3.delete_object(bucket, key) -> Bool
//! - S3.list_objects(bucket, prefix?) -> Array
//! - S3.copy_object(source, dest) -> Bool
//!
//! Credentials are loaded from environment variables:
//! - AWS_ACCESS_KEY_ID or S3_ACCESS_KEY
//! - AWS_SECRET_ACCESS_KEY or S3_SECRET_KEY
//! - AWS_REGION or S3_REGION (default: us-east-1)
//! - S3_ENDPOINT (optional, for MinIO/custom endpoints)

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::rc::Rc;

use rusoto_core::Region;
use rusoto_credential::StaticProvider;
use rusoto_s3::{
    CopyObjectRequest, CreateBucketRequest, DeleteBucketRequest, DeleteObjectRequest,
    GetObjectRequest, ListObjectsV2Request, PutObjectRequest, S3Client, S3,
};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};
use crate::serve::get_tokio_handle;

thread_local! {
    static FALLBACK_RT: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create fallback tokio runtime");
}

fn run_s3_future<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    if let Some(rt) = get_tokio_handle() {
        rt.block_on(future)
    } else {
        FALLBACK_RT.with(|rt| rt.block_on(future))
    }
}

fn scrub_s3_error(error: &str) -> String {
    static SENSITIVE_HEADERS: &[&str] = &[
        "authorization",
        "x-amz-content-sha256",
        "x-amz-date",
        "x-amz-security-token",
    ];

    let lower = error.to_lowercase();
    let mut result = error.to_string();

    for header in SENSITIVE_HEADERS {
        let pattern = format!("{}:", header);
        if let Some(start) = lower.find(&pattern) {
            let end_of_line = result[start..]
                .find('\n')
                .map(|i| start + i)
                .unwrap_or(result.len());

            let line_start = result[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);

            let scrubbed = format!("[scrubbed {}]", header);
            result = format!(
                "{}{}{}",
                &result[..line_start],
                scrubbed,
                &result[end_of_line..]
            );
        }
    }

    for header in SENSITIVE_HEADERS {
        let pattern = format!("\"{}:", header);
        if let Some(start) = result.to_lowercase().find(&pattern) {
            let end = result[start..]
                .find('\"')
                .map(|i| start + i + 1)
                .unwrap_or(result.len());

            let line_start = result[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);

            let scrubbed = format!("\"[scrubbed {}]", header);
            result = format!("{}{}{}", &result[..line_start], scrubbed, &result[end..]);
        }
    }

    result
}

fn extract_string(
    args: &[Value],
    idx: usize,
    fn_name: &str,
    param: &str,
) -> Result<String, String> {
    match args.get(idx) {
        Some(Value::String(s)) => Ok(s.clone().to_string()),
        Some(other) => Err(format!(
            "{}() expects string {}, got {}",
            fn_name,
            param,
            other.type_name()
        )),
        None => Err(format!("{}() missing argument: {}", fn_name, param)),
    }
}

/// SEC-021: validate an S3 bucket name against the conservative subset
/// `^[a-z0-9.-]{3,63}$`. Used by `copy_object` to refuse a `source` like
/// `"my-bucket?versionId=evil/foo"` — the `?versionId=…` would otherwise
/// be concatenated into the `x-amz-copy-source` header value, letting an
/// attacker pivot the copy to a bucket/version they control.
///
/// AWS's full naming rules are stricter (no `..`, no `-.`, no IP-shaped
/// names, must start/end alphanumeric); this regex is the minimum set
/// that blocks header injection. Tightening can come later if needed.
fn validate_bucket_name(name: &str) -> Result<(), String> {
    if name.len() < 3 || name.len() > 63 {
        return Err(format!(
            "Invalid bucket name '{}': length must be 3-63 characters",
            name
        ));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
    {
        return Err(format!(
            "Invalid bucket name '{}': only lowercase letters, digits, '.' and '-' are allowed",
            name
        ));
    }
    Ok(())
}

fn build_s3_client() -> Result<S3Client, String> {
    let access_key = env::var("AWS_ACCESS_KEY_ID")
        .or_else(|_| env::var("S3_ACCESS_KEY"))
        .map_err(|_| "S3_ACCESS_KEY or AWS_ACCESS_KEY_ID not set".to_string())?;

    let secret_key = env::var("AWS_SECRET_ACCESS_KEY")
        .or_else(|_| env::var("S3_SECRET_KEY"))
        .map_err(|_| "S3_SECRET_KEY or AWS_SECRET_ACCESS_KEY not set".to_string())?;

    let region_name = env::var("AWS_REGION")
        .or_else(|_| env::var("S3_REGION"))
        .unwrap_or_else(|_| "us-east-1".to_string());

    let endpoint = env::var("S3_ENDPOINT").ok();

    let region = if let Some(ep) = endpoint {
        Region::Custom {
            name: region_name,
            endpoint: ep,
        }
    } else {
        region_name.parse().unwrap_or(Region::UsEast1)
    };

    let provider = StaticProvider::new(access_key, secret_key, None, None);

    Ok(S3Client::new_with(
        rusoto_core::HttpClient::new().map_err(|e| e.to_string())?,
        provider,
        region,
    ))
}

fn get_s3_client() -> Result<S3Client, String> {
    build_s3_client()
}

pub fn register_s3_class(env: &mut Environment) {
    let mut s3_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    s3_static_methods.insert(
        "list_buckets".to_string(),
        Rc::new(NativeFunction::new("S3.list_buckets", None, |_args| {
            let client = get_s3_client()?;
            run_s3_future(async move {
                match client.list_buckets().await {
                    Ok(result) => {
                        let buckets: Vec<Value> = result
                            .buckets
                            .unwrap_or_default()
                            .into_iter()
                            .map(|b| Value::String(b.name.unwrap_or_default().into()))
                            .collect();
                        Ok(Value::Array(Rc::new(RefCell::new(buckets))))
                    }
                    Err(e) => Err(format!(
                        "Failed to list buckets: {}",
                        scrub_s3_error(&format!("{}", e))
                    )),
                }
            })
        })),
    );

    s3_static_methods.insert(
        "create_bucket".to_string(),
        Rc::new(NativeFunction::new("S3.create_bucket", Some(1), |args| {
            let bucket_name = extract_string(&args, 0, "S3.create_bucket", "bucket name")?;
            let client = get_s3_client()?;
            let request = CreateBucketRequest {
                bucket: bucket_name.clone(),
                ..Default::default()
            };
            run_s3_future(async move {
                match client.create_bucket(request).await {
                    Ok(_) => Ok(Value::Bool(true)),
                    Err(e) => Err(format!(
                        "Failed to create bucket '{}': {}",
                        bucket_name,
                        scrub_s3_error(&format!("{}", e))
                    )),
                }
            })
        })),
    );

    s3_static_methods.insert(
        "delete_bucket".to_string(),
        Rc::new(NativeFunction::new("S3.delete_bucket", Some(1), |args| {
            let bucket_name = extract_string(&args, 0, "S3.delete_bucket", "bucket name")?;
            let client = get_s3_client()?;
            let request = DeleteBucketRequest {
                bucket: bucket_name.clone(),
                ..Default::default()
            };
            run_s3_future(async move {
                match client.delete_bucket(request).await {
                    Ok(_) => Ok(Value::Bool(true)),
                    Err(e) => Err(format!(
                        "Failed to delete bucket '{}': {}",
                        bucket_name,
                        scrub_s3_error(&format!("{}", e))
                    )),
                }
            })
        })),
    );

    s3_static_methods.insert(
        "put_object".to_string(),
        Rc::new(NativeFunction::new("S3.put_object", None, |args| {
            if args.len() < 3 || args.len() > 4 {
                return Err(format!(
                    "S3.put_object() expects 3-4 arguments (bucket, key, body, options?), got {}",
                    args.len()
                ));
            }
            let bucket = extract_string(&args, 0, "S3.put_object", "bucket")?;
            let key = extract_string(&args, 1, "S3.put_object", "key")?;
            let body = extract_string(&args, 2, "S3.put_object", "body")?;

            let mut content_type = "application/octet-stream".to_string();
            if let Some(Value::Hash(options)) = args.get(3) {
                let options = options.borrow();
                let ct_key = HashKey::String("content_type".into());
                if let Some(Value::String(ct)) = options.get(&ct_key) {
                    content_type = ct.clone().to_string();
                }
            }

            let client = get_s3_client()?;
            let request = PutObjectRequest {
                bucket: bucket.clone(),
                key: key.clone(),
                body: Some(body.into_bytes().into()),
                content_type: Some(content_type),
                ..Default::default()
            };
            run_s3_future(async move {
                match client.put_object(request).await {
                    Ok(_) => Ok(Value::Bool(true)),
                    Err(e) => Err(format!(
                        "Failed to put object '{}' in '{}': {}",
                        key,
                        bucket,
                        scrub_s3_error(&format!("{}", e))
                    )),
                }
            })
        })),
    );

    s3_static_methods.insert(
        "get_object".to_string(),
        Rc::new(NativeFunction::new("S3.get_object", Some(2), |args| {
            let bucket = extract_string(&args, 0, "S3.get_object", "bucket")?;
            let key = extract_string(&args, 1, "S3.get_object", "key")?;

            let client = get_s3_client()?;
            let request = GetObjectRequest {
                bucket: bucket.clone(),
                key: key.clone(),
                ..Default::default()
            };
            run_s3_future(async move {
                match client.get_object(request).await {
                    Ok(result) => {
                        use futures_util::StreamExt;
                        let mut body = result.body.ok_or("No body in response")?;
                        let mut bytes = bytes::BytesMut::new();
                        while let Some(Ok(chunk)) = body.next().await {
                            bytes.extend_from_slice(&chunk);
                        }
                        String::from_utf8(bytes.to_vec())
                            .map(|s| Value::String(s.into()))
                            .map_err(|_| {
                                format!(
                                    "Object '{}' in '{}' contains non-UTF-8 binary data",
                                    key, bucket
                                )
                            })
                    }
                    Err(e) => Err(format!(
                        "Failed to get object '{}' from '{}': {}",
                        key,
                        bucket,
                        scrub_s3_error(&format!("{}", e))
                    )),
                }
            })
        })),
    );

    s3_static_methods.insert(
        "delete_object".to_string(),
        Rc::new(NativeFunction::new("S3.delete_object", Some(2), |args| {
            let bucket = extract_string(&args, 0, "S3.delete_object", "bucket")?;
            let key = extract_string(&args, 1, "S3.delete_object", "key")?;

            let client = get_s3_client()?;
            let request = DeleteObjectRequest {
                bucket: bucket.clone(),
                key: key.clone(),
                ..Default::default()
            };
            run_s3_future(async move {
                match client.delete_object(request).await {
                    Ok(_) => Ok(Value::Bool(true)),
                    Err(e) => Err(format!(
                        "Failed to delete object '{}' from '{}': {}",
                        key,
                        bucket,
                        scrub_s3_error(&format!("{}", e))
                    )),
                }
            })
        })),
    );

    s3_static_methods.insert(
        "list_objects".to_string(),
        Rc::new(NativeFunction::new("S3.list_objects", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err(format!(
                    "S3.list_objects() expects 1-2 arguments (bucket, prefix?), got {}",
                    args.len()
                ));
            }
            let bucket = extract_string(&args, 0, "S3.list_objects", "bucket")?;
            let prefix = if args.len() > 1 {
                Some(extract_string(&args, 1, "S3.list_objects", "prefix")?)
            } else {
                None
            };

            let client = get_s3_client()?;
            run_s3_future(async move {
                let mut all_keys = Vec::new();
                let mut continuation_token: Option<String> = None;
                loop {
                    let request = ListObjectsV2Request {
                        bucket: bucket.clone(),
                        prefix: prefix.clone(),
                        continuation_token: continuation_token.take(),
                        ..Default::default()
                    };
                    let result = client
                        .list_objects_v2(request)
                        .await
                        .map_err(|e| format!("Failed to list objects in '{}': {}", bucket, e))?;

                    if let Some(contents) = result.contents {
                        for obj in contents {
                            if let Some(key) = obj.key {
                                all_keys.push(Value::String(key.into()));
                            }
                        }
                    }
                    match result.next_continuation_token {
                        Some(token) if !token.is_empty() => {
                            continuation_token = Some(token);
                        }
                        _ => break,
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(all_keys))))
            })
        })),
    );

    s3_static_methods.insert(
        "copy_object".to_string(),
        Rc::new(NativeFunction::new("S3.copy_object", Some(2), |args| {
            let source = extract_string(&args, 0, "S3.copy_object", "source")?;
            let dest = extract_string(&args, 1, "S3.copy_object", "dest")?;

            let (src_bucket, src_key) = source
                .split_once('/')
                .ok_or("Source must be in format 'bucket/key'")?;
            let (dst_bucket, dst_key) = dest
                .split_once('/')
                .ok_or("Dest must be in format 'bucket/key'")?;

            // SEC-021: refuse bucket names that could inject query-string
            // or other special chars into the `x-amz-copy-source` header.
            // Encode the validated bucket as belt-and-suspenders so any
            // future relaxation of `validate_bucket_name` doesn't reopen
            // the hole.
            validate_bucket_name(src_bucket)?;
            validate_bucket_name(dst_bucket)?;

            let client = get_s3_client()?;
            let copy_source = format!(
                "{}/{}",
                urlencoding::encode(src_bucket),
                urlencoding::encode(src_key)
            );
            let request = CopyObjectRequest {
                bucket: dst_bucket.to_string(),
                key: dst_key.to_string(),
                copy_source,
                ..Default::default()
            };
            run_s3_future(async move {
                match client.copy_object(request).await {
                    Ok(_) => Ok(Value::Bool(true)),
                    Err(e) => Err(format!(
                        "Failed to copy '{}' to '{}': {}",
                        source,
                        dest,
                        scrub_s3_error(&format!("{}", e))
                    )),
                }
            })
        })),
    );

    let s3_class = Class {
        name: "S3".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: s3_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.define("S3".to_string(), Value::Class(Rc::new(s3_class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_bucket_name_accepts_valid() {
        assert!(validate_bucket_name("my-bucket").is_ok());
        assert!(validate_bucket_name("a.b-c.123").is_ok());
        assert!(validate_bucket_name("abc").is_ok()); // min length
        assert!(validate_bucket_name(&"a".repeat(63)).is_ok()); // max length
    }

    #[test]
    fn validate_bucket_name_rejects_injection_attempts() {
        // SEC-021: the original CVE — `?versionId=` injection.
        assert!(validate_bucket_name("my-bucket?versionId=evil").is_err());
        // Other special chars that would land in a header value.
        assert!(validate_bucket_name("my bucket").is_err()); // space
        assert!(validate_bucket_name("my/bucket").is_err()); // slash
        assert!(validate_bucket_name("my\nbucket").is_err()); // CRLF injection
        assert!(validate_bucket_name("MyBucket").is_err()); // uppercase
        assert!(validate_bucket_name("").is_err()); // empty
        assert!(validate_bucket_name("ab").is_err()); // too short
        assert!(validate_bucket_name(&"a".repeat(64)).is_err()); // too long
    }
}
