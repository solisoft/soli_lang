//! S3 built-in class for SoliLang.
//!
//! Provides the S3 class with static methods for S3 operations:
//! - S3.list_buckets() -> Array
//! - S3.create_bucket(name) -> Bool
//! - S3.delete_bucket(name) -> Bool
//! - S3.put_object(bucket, key, body, options?) -> Bool
//! - S3.get_object(bucket, key) -> String
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

pub fn register_s3_class(env: &mut Environment) {
    let mut s3_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    s3_static_methods.insert(
        "list_buckets".to_string(),
        Rc::new(NativeFunction::new("S3.list_buckets", Some(0), |_args| {
            let client = build_s3_client()?;

            match get_tokio_handle() {
                Some(rt) => rt.block_on(async move {
                    match client.list_buckets().await {
                        Ok(result) => {
                            let buckets: Vec<Value> = result
                                .buckets
                                .unwrap_or_default()
                                .into_iter()
                                .map(|b| Value::String(b.name.unwrap_or_default()))
                                .collect();
                            Ok(Value::Array(Rc::new(RefCell::new(buckets))))
                        }
                        Err(e) => Err(format!("Failed to list buckets: {}", e)),
                    }
                }),
                None => Err("No tokio runtime available".to_string()),
            }
        })),
    );

    s3_static_methods.insert(
        "create_bucket".to_string(),
        Rc::new(NativeFunction::new("S3.create_bucket", Some(1), |args| {
            let bucket_name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.create_bucket() expects string bucket name, got {}",
                        other.type_name()
                    ))
                }
            };

            let client = build_s3_client()?;
            let request = CreateBucketRequest {
                bucket: bucket_name.clone(),
                ..Default::default()
            };

            match get_tokio_handle() {
                Some(rt) => rt.block_on(async move {
                    match client.create_bucket(request).await {
                        Ok(_) => Ok(Value::Bool(true)),
                        Err(e) => Err(format!("Failed to create bucket '{}': {}", bucket_name, e)),
                    }
                }),
                None => Err("No tokio runtime available".to_string()),
            }
        })),
    );

    s3_static_methods.insert(
        "delete_bucket".to_string(),
        Rc::new(NativeFunction::new("S3.delete_bucket", Some(1), |args| {
            let bucket_name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.delete_bucket() expects string bucket name, got {}",
                        other.type_name()
                    ))
                }
            };

            let client = build_s3_client()?;
            let request = DeleteBucketRequest {
                bucket: bucket_name.clone(),
                ..Default::default()
            };

            match get_tokio_handle() {
                Some(rt) => rt.block_on(async move {
                    match client.delete_bucket(request).await {
                        Ok(_) => Ok(Value::Bool(true)),
                        Err(e) => Err(format!("Failed to delete bucket '{}': {}", bucket_name, e)),
                    }
                }),
                None => Err("No tokio runtime available".to_string()),
            }
        })),
    );

    s3_static_methods.insert(
        "put_object".to_string(),
        Rc::new(NativeFunction::new("S3.put_object", Some(3), |args| {
            let bucket = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.put_object() expects string bucket, got {}",
                        other.type_name()
                    ))
                }
            };

            let key = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.put_object() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let body = match &args[2] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.put_object() expects string body, got {}",
                        other.type_name()
                    ))
                }
            };

            let mut content_type = "application/octet-stream".to_string();
            if args.len() > 3 {
                if let Value::Hash(options) = &args[3] {
                    let options = options.borrow();
                    let ct_key = HashKey::String("content_type".to_string());
                    if let Some(Value::String(ct)) = options.get(&ct_key) {
                        content_type = ct.clone();
                    }
                }
            }

            let client = build_s3_client()?;
            let request = PutObjectRequest {
                bucket: bucket.clone(),
                key: key.clone(),
                body: Some(body.into_bytes().into()),
                content_type: Some(content_type),
                ..Default::default()
            };

            match get_tokio_handle() {
                Some(rt) => rt.block_on(async move {
                    match client.put_object(request).await {
                        Ok(_) => Ok(Value::Bool(true)),
                        Err(e) => Err(format!(
                            "Failed to put object '{}' in '{}': {}",
                            key, bucket, e
                        )),
                    }
                }),
                None => Err("No tokio runtime available".to_string()),
            }
        })),
    );

    s3_static_methods.insert(
        "get_object".to_string(),
        Rc::new(NativeFunction::new("S3.get_object", Some(2), |args| {
            let bucket = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.get_object() expects string bucket, got {}",
                        other.type_name()
                    ))
                }
            };

            let key = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.get_object() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let client = build_s3_client()?;
            let request = GetObjectRequest {
                bucket: bucket.clone(),
                key: key.clone(),
                ..Default::default()
            };

            match get_tokio_handle() {
                Some(rt) => rt.block_on(async move {
                    match client.get_object(request).await {
                        Ok(result) => {
                            use futures_util::StreamExt;
                            let mut body = result.body.ok_or("No body in response")?;
                            let mut bytes = bytes::BytesMut::new();
                            while let Some(Ok(chunk)) = body.next().await {
                                bytes.extend_from_slice(&chunk);
                            }
                            Ok(Value::String(String::from_utf8_lossy(&bytes).to_string()))
                        }
                        Err(e) => Err(format!(
                            "Failed to get object '{}' from '{}': {}",
                            key, bucket, e
                        )),
                    }
                }),
                None => Err("No tokio runtime available".to_string()),
            }
        })),
    );

    s3_static_methods.insert(
        "delete_object".to_string(),
        Rc::new(NativeFunction::new("S3.delete_object", Some(2), |args| {
            let bucket = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.delete_object() expects string bucket, got {}",
                        other.type_name()
                    ))
                }
            };

            let key = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.delete_object() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let client = build_s3_client()?;
            let request = DeleteObjectRequest {
                bucket: bucket.clone(),
                key: key.clone(),
                ..Default::default()
            };

            match get_tokio_handle() {
                Some(rt) => rt.block_on(async move {
                    match client.delete_object(request).await {
                        Ok(_) => Ok(Value::Bool(true)),
                        Err(e) => Err(format!(
                            "Failed to delete object '{}' from '{}': {}",
                            key, bucket, e
                        )),
                    }
                }),
                None => Err("No tokio runtime available".to_string()),
            }
        })),
    );

    s3_static_methods.insert(
        "list_objects".to_string(),
        Rc::new(NativeFunction::new("S3.list_objects", Some(1), |args| {
            let bucket = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.list_objects() expects string bucket, got {}",
                        other.type_name()
                    ))
                }
            };

            let prefix = if args.len() > 1 {
                if let Value::String(s) = &args[1] {
                    Some(s.clone())
                } else {
                    None
                }
            } else {
                None
            };

            let client = build_s3_client()?;
            let request = ListObjectsV2Request {
                bucket: bucket.clone(),
                prefix: prefix.clone(),
                ..Default::default()
            };

            match get_tokio_handle() {
                Some(rt) => rt.block_on(async move {
                    match client.list_objects_v2(request).await {
                        Ok(result) => {
                            let keys: Vec<Value> = result
                                .contents
                                .unwrap_or_default()
                                .into_iter()
                                .map(|obj| Value::String(obj.key.unwrap_or_default()))
                                .collect();
                            Ok(Value::Array(Rc::new(RefCell::new(keys))))
                        }
                        Err(e) => Err(format!("Failed to list objects in '{}': {}", bucket, e)),
                    }
                }),
                None => Err("No tokio runtime available".to_string()),
            }
        })),
    );

    s3_static_methods.insert(
        "copy_object".to_string(),
        Rc::new(NativeFunction::new("S3.copy_object", Some(2), |args| {
            let source = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.copy_object() expects string source, got {}",
                        other.type_name()
                    ))
                }
            };

            let dest = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "S3.copy_object() expects string dest, got {}",
                        other.type_name()
                    ))
                }
            };

            let (src_bucket, src_key) = source
                .split_once('/')
                .ok_or("Source must be in format 'bucket/key'")?;
            let (dst_bucket, dst_key) = dest
                .split_once('/')
                .ok_or("Dest must be in format 'bucket/key'")?;

            let client = build_s3_client()?;
            let copy_source = format!("{}/{}", src_bucket, src_key);
            let request = CopyObjectRequest {
                bucket: dst_bucket.to_string(),
                key: dst_key.to_string(),
                copy_source,
                ..Default::default()
            };

            match get_tokio_handle() {
                Some(rt) => rt.block_on(async move {
                    match client.copy_object(request).await {
                        Ok(_) => Ok(Value::Bool(true)),
                        Err(e) => Err(format!("Failed to copy '{}' to '{}': {}", source, dest, e)),
                    }
                }),
                None => Err("No tokio runtime available".to_string()),
            }
        })),
    );

    let s3_class = Class {
        name: "S3".to_string(),
        superclass: None,
        methods: HashMap::new(),
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
