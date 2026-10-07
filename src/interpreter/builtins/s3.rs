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
use std::rc::Rc;

use super::args::string_arg as extract_string;
use super::s3_client::{run, S3Client};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};

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

pub fn register_s3_class(env: &mut Environment) {
    let mut s3_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    s3_static_methods.insert(
        "list_buckets".to_string(),
        Rc::new(NativeFunction::new("S3.list_buckets", None, |_args| {
            let client = S3Client::from_env()?;
            let names =
                run(client.list_buckets()).map_err(|e| format!("Failed to list buckets: {}", e))?;
            let buckets: Vec<Value> = names.into_iter().map(|n| Value::String(n.into())).collect();
            Ok(Value::Array(Rc::new(RefCell::new(buckets))))
        })),
    );

    s3_static_methods.insert(
        "create_bucket".to_string(),
        Rc::new(NativeFunction::new("S3.create_bucket", Some(1), |args| {
            let bucket_name = extract_string(args, 0, "S3.create_bucket", "bucket name")?;
            let client = S3Client::from_env()?;
            run(client.create_bucket(&bucket_name))
                .map_err(|e| format!("Failed to create bucket '{}': {}", bucket_name, e))?;
            Ok(Value::Bool(true))
        })),
    );

    s3_static_methods.insert(
        "delete_bucket".to_string(),
        Rc::new(NativeFunction::new("S3.delete_bucket", Some(1), |args| {
            let bucket_name = extract_string(args, 0, "S3.delete_bucket", "bucket name")?;
            let client = S3Client::from_env()?;
            run(client.delete_bucket(&bucket_name))
                .map_err(|e| format!("Failed to delete bucket '{}': {}", bucket_name, e))?;
            Ok(Value::Bool(true))
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
            let bucket = extract_string(args, 0, "S3.put_object", "bucket")?;
            let key = extract_string(args, 1, "S3.put_object", "key")?;
            let body = extract_string(args, 2, "S3.put_object", "body")?;

            let mut content_type = "application/octet-stream".to_string();
            if let Some(Value::Hash(options)) = args.get(3) {
                let options = options.borrow();
                let ct_key = HashKey::String("content_type".into());
                if let Some(Value::String(ct)) = options.get(&ct_key) {
                    content_type = ct.clone().to_string();
                }
            }

            let client = S3Client::from_env()?;
            run(client.put_object(&bucket, &key, body.into_bytes().into(), &content_type, &[]))
                .map_err(|e| format!("Failed to put object '{}' in '{}': {}", key, bucket, e))?;
            Ok(Value::Bool(true))
        })),
    );

    s3_static_methods.insert(
        "get_object".to_string(),
        Rc::new(NativeFunction::new("S3.get_object", Some(2), |args| {
            let bucket = extract_string(args, 0, "S3.get_object", "bucket")?;
            let key = extract_string(args, 1, "S3.get_object", "key")?;

            let client = S3Client::from_env()?;
            let (_, bytes) = run(client.get_object(&bucket, &key))
                .map_err(|e| format!("Failed to get object '{}' from '{}': {}", key, bucket, e))?;
            String::from_utf8(bytes.to_vec())
                .map(|s| Value::String(s.into()))
                .map_err(|_| {
                    format!(
                        "Object '{}' in '{}' contains non-UTF-8 binary data",
                        key, bucket
                    )
                })
        })),
    );

    s3_static_methods.insert(
        "delete_object".to_string(),
        Rc::new(NativeFunction::new("S3.delete_object", Some(2), |args| {
            let bucket = extract_string(args, 0, "S3.delete_object", "bucket")?;
            let key = extract_string(args, 1, "S3.delete_object", "key")?;

            let client = S3Client::from_env()?;
            run(client.delete_object(&bucket, &key)).map_err(|e| {
                format!("Failed to delete object '{}' from '{}': {}", key, bucket, e)
            })?;
            Ok(Value::Bool(true))
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
            let bucket = extract_string(args, 0, "S3.list_objects", "bucket")?;
            let prefix = if args.len() > 1 {
                Some(extract_string(args, 1, "S3.list_objects", "prefix")?)
            } else {
                None
            };

            let client = S3Client::from_env()?;
            let keys = run(async {
                let mut all_keys = Vec::new();
                let mut token: Option<String> = None;
                loop {
                    let page = client
                        .list_objects_page(&bucket, prefix.as_deref(), token.as_deref())
                        .await?;
                    all_keys.extend(page.keys.into_iter().map(|k| Value::String(k.into())));
                    match page.next_token {
                        Some(next) => token = Some(next),
                        None => break,
                    }
                }
                Ok::<_, String>(all_keys)
            })
            .map_err(|e| format!("Failed to list objects in '{}': {}", bucket, e))?;
            Ok(Value::Array(Rc::new(RefCell::new(keys))))
        })),
    );

    s3_static_methods.insert(
        "copy_object".to_string(),
        Rc::new(NativeFunction::new("S3.copy_object", Some(2), |args| {
            let source = extract_string(args, 0, "S3.copy_object", "source")?;
            let dest = extract_string(args, 1, "S3.copy_object", "dest")?;

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

            let client = S3Client::from_env()?;
            let copy_source = format!(
                "{}/{}",
                urlencoding::encode(src_bucket),
                urlencoding::encode(src_key)
            );
            run(client.copy_object(&copy_source, dst_bucket, dst_key))
                .map_err(|e| format!("Failed to copy '{}' to '{}': {}", source, dest, e))?;
            Ok(Value::Bool(true))
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
