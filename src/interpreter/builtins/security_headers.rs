use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};
use indexmap::IndexMap;
use lazy_static::lazy_static;
use std::sync::RwLock;

lazy_static! {
    static ref SECURITY_HEADERS_CONFIG: RwLock<SecurityHeadersConfig> =
        RwLock::new(SecurityHeadersConfig::default());
    static ref SECURITY_HEADERS_ENABLED: RwLock<bool> = RwLock::new(false);
}

#[derive(Clone, Default)]
struct SecurityHeadersConfig {
    csp: Option<String>,
    csp_report_only: Option<String>,
    hsts: Option<HstsConfig>,
    x_frame_options: Option<String>,
    x_content_type_options: bool,
    xss_protection: Option<String>,
    referrer_policy: Option<String>,
    permissions_policy: Option<String>,
    cross_origin_embedder_policy: Option<String>,
    cross_origin_opener_policy: Option<String>,
    cross_origin_resource_policy: Option<String>,
}

#[derive(Clone)]
struct HstsConfig {
    max_age: u64,
    include_subdomains: bool,
    preload: bool,
}

pub fn register_security_headers_builtins(env: &mut Environment) {
    env.define(
        "enable_security_headers".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "enable_security_headers",
            Some(0),
            |_args| {
                let mut enabled = SECURITY_HEADERS_ENABLED
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                *enabled = true;
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "disable_security_headers".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "disable_security_headers",
            Some(0),
            |_args| {
                let mut enabled = SECURITY_HEADERS_ENABLED
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                *enabled = false;
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "security_headers_enabled".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "security_headers_enabled",
            Some(0),
            |_args| {
                let enabled = SECURITY_HEADERS_ENABLED
                    .read()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                Ok(Value::Bool(*enabled))
            },
        )),
    );

    env.define(
        "set_csp".to_string(),
        Value::NativeFunction(NativeFunction::new("set_csp", Some(1), |args| {
            let policy = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "set_csp() expects string policy, got {}",
                        other.type_name()
                    ))
                }
            };
            let report_only = args.get(1).map(|v| v.is_truthy()).unwrap_or(false);

            let mut config = SECURITY_HEADERS_CONFIG
                .write()
                .map_err(|e| format!("Security headers error: {}", e))?;
            if report_only {
                config.csp_report_only = Some(policy);
            } else {
                config.csp = Some(policy);
            }
            Ok(Value::Null)
        })),
    );

    env.define(
        "set_csp_default_src".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "set_csp_default_src",
            Some(1),
            |args| {
                let sources: Vec<String> = args
                    .iter()
                    .filter_map(|v| {
                        if let Value::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                let policy = format!("default-src {}", sources.join(" "));
                let mut config = SECURITY_HEADERS_CONFIG
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                config.csp = Some(policy);
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "set_csp_script_src".to_string(),
        Value::NativeFunction(NativeFunction::new("set_csp_script_src", Some(1), |args| {
            let sources: Vec<String> = args
                .iter()
                .filter_map(|v| {
                    if let Value::String(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let policy = format!("script-src {}", sources.join(" "));
            let mut config = SECURITY_HEADERS_CONFIG
                .write()
                .map_err(|e| format!("Security headers error: {}", e))?;
            config.csp = Some(policy);
            Ok(Value::Null)
        })),
    );

    env.define(
        "set_csp_style_src".to_string(),
        Value::NativeFunction(NativeFunction::new("set_csp_style_src", Some(1), |args| {
            let sources: Vec<String> = args
                .iter()
                .filter_map(|v| {
                    if let Value::String(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let policy = format!("style-src {}", sources.join(" "));
            let mut config = SECURITY_HEADERS_CONFIG
                .write()
                .map_err(|e| format!("Security headers error: {}", e))?;
            config.csp = Some(policy);
            Ok(Value::Null)
        })),
    );

    env.define(
        "set_hsts".to_string(),
        Value::NativeFunction(NativeFunction::new("set_hsts", Some(1), |args| {
            let max_age = match &args[0] {
                Value::Int(i) => *i as u64,
                other => {
                    return Err(format!(
                        "set_hsts() expects int max_age, got {}",
                        other.type_name()
                    ))
                }
            };
            let include_subdomains = args.get(1).map(|v| v.is_truthy()).unwrap_or(true);
            let preload = args.get(2).map(|v| v.is_truthy()).unwrap_or(false);

            let mut config = SECURITY_HEADERS_CONFIG
                .write()
                .map_err(|e| format!("Security headers error: {}", e))?;
            config.hsts = Some(HstsConfig {
                max_age,
                include_subdomains,
                preload,
            });
            Ok(Value::Null)
        })),
    );

    env.define(
        "prevent_clickjacking".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "prevent_clickjacking",
            Some(0),
            |_args| {
                let mut config = SECURITY_HEADERS_CONFIG
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                config.x_frame_options = Some("DENY".to_string());
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "allow_same_origin_frames".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "allow_same_origin_frames",
            Some(0),
            |_args| {
                let mut config = SECURITY_HEADERS_CONFIG
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                config.x_frame_options = Some("SAMEORIGIN".to_string());
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "set_xss_protection".to_string(),
        Value::NativeFunction(NativeFunction::new("set_xss_protection", Some(1), |args| {
            let mode = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "set_xss_protection() expects string mode, got {}",
                        other.type_name()
                    ))
                }
            };
            let mut config = SECURITY_HEADERS_CONFIG
                .write()
                .map_err(|e| format!("Security headers error: {}", e))?;
            config.xss_protection = Some(format!("1; mode={}", mode));
            Ok(Value::Null)
        })),
    );

    env.define(
        "set_content_type_options".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "set_content_type_options",
            Some(0),
            |_args| {
                let mut config = SECURITY_HEADERS_CONFIG
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                config.x_content_type_options = true;
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "set_referrer_policy".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "set_referrer_policy",
            Some(1),
            |args| {
                let policy = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "set_referrer_policy() expects string policy, got {}",
                            other.type_name()
                        ))
                    }
                };
                let mut config = SECURITY_HEADERS_CONFIG
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                config.referrer_policy = Some(policy);
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "set_permissions_policy".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "set_permissions_policy",
            Some(1),
            |args| {
                let policy = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "set_permissions_policy() expects string policy, got {}",
                            other.type_name()
                        ))
                    }
                };
                let mut config = SECURITY_HEADERS_CONFIG
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                config.permissions_policy = Some(policy);
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "set_coep".to_string(),
        Value::NativeFunction(NativeFunction::new("set_coep", Some(1), |args| {
            let policy = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "set_coep() expects string policy, got {}",
                        other.type_name()
                    ))
                }
            };
            let mut config = SECURITY_HEADERS_CONFIG
                .write()
                .map_err(|e| format!("Security headers error: {}", e))?;
            config.cross_origin_embedder_policy = Some(policy);
            Ok(Value::Null)
        })),
    );

    env.define(
        "set_coop".to_string(),
        Value::NativeFunction(NativeFunction::new("set_coop", Some(1), |args| {
            let policy = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "set_coop() expects string policy, got {}",
                        other.type_name()
                    ))
                }
            };
            let mut config = SECURITY_HEADERS_CONFIG
                .write()
                .map_err(|e| format!("Security headers error: {}", e))?;
            config.cross_origin_opener_policy = Some(policy);
            Ok(Value::Null)
        })),
    );

    env.define(
        "set_corp".to_string(),
        Value::NativeFunction(NativeFunction::new("set_corp", Some(1), |args| {
            let policy = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "set_corp() expects string policy, got {}",
                        other.type_name()
                    ))
                }
            };
            let mut config = SECURITY_HEADERS_CONFIG
                .write()
                .map_err(|e| format!("Security headers error: {}", e))?;
            config.cross_origin_resource_policy = Some(policy);
            Ok(Value::Null)
        })),
    );

    env.define(
        "secure_headers".to_string(),
        Value::NativeFunction(NativeFunction::new("secure_headers", Some(0), |_args| {
            let mut config = SECURITY_HEADERS_CONFIG
                .write()
                .map_err(|e| format!("Security headers error: {}", e))?;
            config.x_frame_options = Some("SAMEORIGIN".to_string());
            config.x_content_type_options = true;
            config.referrer_policy = Some("strict-origin-when-cross-origin".to_string());
            config.permissions_policy =
                Some("geolocation=(), microphone=(), camera=()".to_string());
            Ok(Value::Null)
        })),
    );

    env.define(
        "secure_headers_basic".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "secure_headers_basic",
            Some(0),
            |_args| {
                let mut config = SECURITY_HEADERS_CONFIG
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                config.x_frame_options = Some("SAMEORIGIN".to_string());
                config.x_content_type_options = true;
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "secure_headers_strict".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "secure_headers_strict",
            Some(0),
            |_args| {
                let mut config = SECURITY_HEADERS_CONFIG
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                config.csp = Some(
                    "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'"
                        .to_string(),
                );
                config.hsts = Some(HstsConfig {
                    max_age: 31536000,
                    include_subdomains: true,
                    preload: false,
                });
                config.x_frame_options = Some("DENY".to_string());
                config.x_content_type_options = true;
                config.referrer_policy = Some("strict-origin".to_string());
                config.permissions_policy =
                    Some("geolocation=(), microphone=(), camera=()".to_string());
                config.cross_origin_embedder_policy = Some("require-corp".to_string());
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "secure_headers_api".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "secure_headers_api",
            Some(0),
            |_args| {
                let mut config = SECURITY_HEADERS_CONFIG
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                config.x_content_type_options = true;
                config.referrer_policy = Some("strict-origin".to_string());
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "reset_security_headers".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "reset_security_headers",
            Some(0),
            |_args| {
                let mut config = SECURITY_HEADERS_CONFIG
                    .write()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                *config = SecurityHeadersConfig::default();
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "get_security_headers".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "get_security_headers",
            Some(0),
            |_args| {
                let config = SECURITY_HEADERS_CONFIG
                    .read()
                    .map_err(|e| format!("Security headers error: {}", e))?;
                let mut headers: IndexMap<HashKey, Value> = IndexMap::new();

                if let Some(ref csp) = config.csp {
                    headers.insert(
                        HashKey::String("Content-Security-Policy".to_string()),
                        Value::String(csp.clone()),
                    );
                }
                if let Some(ref csp_ro) = config.csp_report_only {
                    headers.insert(
                        HashKey::String("Content-Security-Policy-Report-Only".to_string()),
                        Value::String(csp_ro.clone()),
                    );
                }
                if let Some(ref hsts) = config.hsts {
                    let mut hsts_val = format!("max-age={}", hsts.max_age);
                    if hsts.include_subdomains {
                        hsts_val.push_str("; includeSubDomains");
                    }
                    if hsts.preload {
                        hsts_val.push_str("; preload");
                    }
                    headers.insert(
                        HashKey::String("Strict-Transport-Security".to_string()),
                        Value::String(hsts_val),
                    );
                }
                if let Some(ref xfo) = config.x_frame_options {
                    headers.insert(
                        HashKey::String("X-Frame-Options".to_string()),
                        Value::String(xfo.clone()),
                    );
                }
                if config.x_content_type_options {
                    headers.insert(
                        HashKey::String("X-Content-Type-Options".to_string()),
                        Value::String("nosniff".to_string()),
                    );
                }
                if let Some(ref xss) = config.xss_protection {
                    headers.insert(
                        HashKey::String("X-XSS-Protection".to_string()),
                        Value::String(xss.clone()),
                    );
                }
                if let Some(ref rp) = config.referrer_policy {
                    headers.insert(
                        HashKey::String("Referrer-Policy".to_string()),
                        Value::String(rp.clone()),
                    );
                }
                if let Some(ref pp) = config.permissions_policy {
                    headers.insert(
                        HashKey::String("Permissions-Policy".to_string()),
                        Value::String(pp.clone()),
                    );
                }
                if let Some(ref coep) = config.cross_origin_embedder_policy {
                    headers.insert(
                        HashKey::String("Cross-Origin-Embedder-Policy".to_string()),
                        Value::String(coep.clone()),
                    );
                }
                if let Some(ref coop) = config.cross_origin_opener_policy {
                    headers.insert(
                        HashKey::String("Cross-Origin-Opener-Policy".to_string()),
                        Value::String(coop.clone()),
                    );
                }
                if let Some(ref corp) = config.cross_origin_resource_policy {
                    headers.insert(
                        HashKey::String("Cross-Origin-Resource-Policy".to_string()),
                        Value::String(corp.clone()),
                    );
                }

                Ok(Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
                    headers,
                ))))
            },
        )),
    );
}

pub fn get_security_headers() -> Vec<(String, String)> {
    let enabled = match SECURITY_HEADERS_ENABLED.read() {
        Ok(guard) => *guard,
        Err(_) => return Vec::new(),
    };

    if !enabled {
        return Vec::new();
    }

    let config = match SECURITY_HEADERS_CONFIG.read() {
        Ok(guard) => guard.clone(),
        Err(_) => return Vec::new(),
    };

    let mut headers: Vec<(String, String)> = Vec::new();

    if let Some(csp) = config.csp {
        headers.push(("Content-Security-Policy".to_string(), csp));
    }
    if let Some(csp_ro) = config.csp_report_only {
        headers.push(("Content-Security-Policy-Report-Only".to_string(), csp_ro));
    }
    if let Some(hsts) = config.hsts {
        let mut hsts_val = format!("max-age={}", hsts.max_age);
        if hsts.include_subdomains {
            hsts_val.push_str("; includeSubDomains");
        }
        if hsts.preload {
            hsts_val.push_str("; preload");
        }
        headers.push(("Strict-Transport-Security".to_string(), hsts_val));
    }
    if let Some(xfo) = config.x_frame_options {
        headers.push(("X-Frame-Options".to_string(), xfo));
    }
    if config.x_content_type_options {
        headers.push(("X-Content-Type-Options".to_string(), "nosniff".to_string()));
    }
    if let Some(xss) = config.xss_protection {
        headers.push(("X-XSS-Protection".to_string(), xss));
    }
    if let Some(rp) = config.referrer_policy {
        headers.push(("Referrer-Policy".to_string(), rp));
    }
    if let Some(pp) = config.permissions_policy {
        headers.push(("Permissions-Policy".to_string(), pp));
    }
    if let Some(coep) = config.cross_origin_embedder_policy {
        headers.push(("Cross-Origin-Embedder-Policy".to_string(), coep));
    }
    if let Some(coop) = config.cross_origin_opener_policy {
        headers.push(("Cross-Origin-Opener-Policy".to_string(), coop));
    }
    if let Some(corp) = config.cross_origin_resource_policy {
        headers.push(("Cross-Origin-Resource-Policy".to_string(), corp));
    }

    headers
}

pub fn security_headers_enabled() -> bool {
    match SECURITY_HEADERS_ENABLED.read() {
        Ok(guard) => *guard,
        Err(_) => false,
    }
}
