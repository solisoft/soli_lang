//! `AppLinks` — the two files a host must serve for a deep link to *verify*.
//!
//! A shell can declare that it handles `https://example.com/…`, but the OS will
//! not honour that on trust: it fetches a file from the host and checks the app
//! is named in it. Get the file wrong — a stray comma, the wrong fingerprint, a
//! redirect, the wrong content type — and the link silently falls back to the
//! browser with no error anywhere. That hand-written JSON is the entire
//! difficulty of deep links, so this generates it.
//!
//! Two files, one per platform:
//!
//! * **Android** — `/.well-known/assetlinks.json`, listing the app's package
//!   and its signing certificate's SHA-256 fingerprint.
//! * **Apple** — `/.well-known/apple-app-site-association`, listing the app's
//!   `TEAMID.bundle.id` and the path patterns it claims. Served **as
//!   `application/json`, with no `.json` extension and no redirect** — Apple's
//!   CDN fetches it verbatim and a redirect fails verification.
//!
//! ```soli
//! # config/routes.sl
//! get("/.well-known/assetlinks.json", "well_known#android")
//! get("/.well-known/apple-app-site-association", "well_known#apple")
//!
//! # app/controllers/well_known_controller.sl
//! def android(req)
//!   { "headers": { "Content-Type": "application/json" },
//!     "body": AppLinks.android("net.example.app", [ENV["ANDROID_CERT_SHA256"]]) }
//! end
//!
//! def apple(req)
//!   { "headers": { "Content-Type": "application/json" },
//!     "body": AppLinks.apple("ABCDE12345.net.example.app", ["/pings/*", "/threads/*"]) }
//! end
//! ```

use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

/// A package name / app id must be a dotted reverse-DNS identifier, and a
/// certificate fingerprint colon-separated hex. Anything else is a mistake that
/// would produce a file the OS silently rejects, so it is caught here where the
/// error is legible rather than in a verification log the developer never sees.
fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn strings(value: &Value, function: &str, name: &str) -> Result<Vec<String>, String> {
    match value {
        Value::String(s) => Ok(vec![s.to_string()]),
        Value::Array(items) => items
            .borrow()
            .iter()
            .map(|item| match item {
                Value::String(s) => Ok(s.to_string()),
                other => Err(format!(
                    "{}(): every {} must be a string, got {}",
                    function,
                    name,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "{}(): {} must be a string or an array of strings, got {}",
            function,
            name,
            other.type_name()
        )),
    }
}

/// The Android Digital Asset Links statement list.
fn android(package: &str, fingerprints: &[String]) -> Result<String, String> {
    if !valid_identifier(package) {
        return Err(format!(
            "AppLinks.android(): '{}' is not a valid package name",
            package
        ));
    }
    if fingerprints.is_empty() {
        return Err(
            "AppLinks.android(): at least one SHA-256 certificate fingerprint is required \
             (from `keytool -list -v -keystore …`, or the Play Console)"
                .to_string(),
        );
    }
    // Normalize each fingerprint to the upper-case colon-separated form Google
    // matches on; accept the plain-hex form a developer might paste.
    let normalized: Result<Vec<String>, String> = fingerprints
        .iter()
        .map(|raw| normalize_fingerprint(raw))
        .collect();

    let statement = serde_json::json!([{
        "relation": ["delegate_permission/common.handle_all_urls"],
        "target": {
            "namespace": "android_app",
            "package_name": package,
            "sha256_cert_fingerprints": normalized?,
        }
    }]);
    serde_json::to_string_pretty(&statement).map_err(|e| format!("AppLinks.android(): {}", e))
}

/// SHA-256 fingerprints are 32 bytes = 64 hex chars, canonically upper-case and
/// colon-separated. Accept either form; reject anything that is not 32 bytes,
/// because a wrong-length fingerprint verifies against nothing.
fn normalize_fingerprint(raw: &str) -> Result<String, String> {
    let hex: String = raw.chars().filter(|c| *c != ':').collect();
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "AppLinks.android(): '{}' is not a SHA-256 fingerprint (expected 32 bytes / 64 hex chars)",
            raw
        ));
    }
    let upper = hex.to_ascii_uppercase();
    let pairs: Vec<String> = (0..64)
        .step_by(2)
        .map(|i| upper[i..i + 2].to_string())
        .collect();
    Ok(pairs.join(":"))
}

/// The Apple App Site Association document.
fn apple(app_id: &str, paths: &[String]) -> Result<String, String> {
    // `TEAMID.bundle.id`: the team prefix plus the reverse-DNS bundle id.
    let (team, bundle) = app_id.split_once('.').ok_or_else(|| {
        format!(
            "AppLinks.apple(): '{}' must be 'TEAMID.bundle.id' (the team prefix, then the bundle id)",
            app_id
        )
    })?;
    if team.is_empty() || !valid_identifier(bundle) {
        return Err(format!(
            "AppLinks.apple(): '{}' is not a valid app id",
            app_id
        ));
    }

    // Default to the whole site when no paths are given.
    let patterns: Vec<String> = if paths.is_empty() {
        vec!["*".to_string()]
    } else {
        paths.to_vec()
    };
    // The modern `components` form; `paths` is also emitted for older systems
    // that predate it, so one document serves every OS version.
    let components: Vec<serde_json::Value> = patterns
        .iter()
        .map(|p| serde_json::json!({ "/": p }))
        .collect();

    let association = serde_json::json!({
        "applinks": {
            "apps": [],
            "details": [{
                "appID": app_id,
                "appIDs": [app_id],
                "paths": patterns,
                "components": components,
            }]
        }
    });
    serde_json::to_string_pretty(&association).map_err(|e| format!("AppLinks.apple(): {}", e))
}

pub fn register_app_links_builtins(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // AppLinks.android(package_name, fingerprints) -> String
    statics.insert(
        "android".to_string(),
        Rc::new(NativeFunction::new("AppLinks.android", Some(2), |args| {
            let package = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "AppLinks.android() expects a string package name, got {}",
                        other.type_name()
                    ))
                }
            };
            let fingerprints = strings(&args[1], "AppLinks.android", "fingerprint")?;
            android(&package, &fingerprints).map(|json| Value::String(json.into()))
        })),
    );

    // AppLinks.apple(app_id, paths?) -> String
    statics.insert(
        "apple".to_string(),
        Rc::new(NativeFunction::new("AppLinks.apple", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err("AppLinks.apple() expects (app_id, paths?)".to_string());
            }
            let app_id = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "AppLinks.apple() expects a string app id, got {}",
                        other.type_name()
                    ))
                }
            };
            let paths = match args.get(1) {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => strings(value, "AppLinks.apple", "path")?,
            };
            apple(&app_id, &paths).map(|json| Value::String(json.into()))
        })),
    );

    let class = Rc::new(Class {
        name: "AppLinks".to_string(),
        superclass: None,
        methods: Rc::new(std::cell::RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: statics,
        native_methods: HashMap::new(),
        static_fields: Rc::new(std::cell::RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(std::cell::RefCell::new(HashMap::new())),
        ..Default::default()
    });

    env.define("AppLinks".to_string(), Value::Class(class));
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "14:6D:E9:83:C5:73:06:50:D8:EE:B9:95:2F:34:FC:64:16:A0:83:42:E6:1D:BE:A8:8A:04:96:B2:3F:CF:44:E5";

    #[test]
    fn android_statement_names_the_package_and_fingerprint() {
        let json = android("net.example.app", &[FP.to_string()]).expect("builds");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["target"]["package_name"], "net.example.app");
        assert_eq!(parsed[0]["target"]["namespace"], "android_app");
        assert_eq!(parsed[0]["target"]["sha256_cert_fingerprints"][0], FP);
        assert_eq!(
            parsed[0]["relation"][0],
            "delegate_permission/common.handle_all_urls"
        );
    }

    /// A developer will paste the fingerprint as plain hex; it must come out in
    /// the colon-separated upper-case form Google matches, or verification
    /// silently fails.
    #[test]
    fn a_plain_hex_fingerprint_is_normalized() {
        let plain = FP.replace(':', "").to_lowercase();
        let json = android("net.example.app", &[plain]).expect("builds");
        assert!(
            json.contains(FP),
            "fingerprint should be normalized to {}",
            FP
        );
    }

    #[test]
    fn a_wrong_length_fingerprint_is_rejected() {
        assert!(android("net.example.app", &["AB:CD".to_string()]).is_err());
        assert!(android("net.example.app", &[]).is_err());
    }

    #[test]
    fn an_invalid_package_is_rejected() {
        assert!(android("not a package", &[FP.to_string()]).is_err());
    }

    #[test]
    fn apple_association_carries_the_app_id_and_paths() {
        let json = apple("ABCDE12345.net.example.app", &["/pings/*".to_string()]).expect("builds");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let detail = &parsed["applinks"]["details"][0];
        assert_eq!(detail["appID"], "ABCDE12345.net.example.app");
        assert_eq!(detail["paths"][0], "/pings/*");
        // The modern components form is emitted alongside the legacy paths.
        assert_eq!(detail["components"][0]["/"], "/pings/*");
    }

    #[test]
    fn apple_defaults_to_the_whole_site() {
        let json = apple("ABCDE12345.net.example.app", &[]).expect("builds");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["applinks"]["details"][0]["paths"][0], "*");
    }

    /// An app id must carry a team prefix, i.e. at least one dot. A bare token
    /// cannot be split into `TEAMID.bundle` and is refused; the format itself
    /// is all that can be checked, since a team prefix is indistinguishable
    /// from a leading bundle segment.
    #[test]
    fn an_app_id_without_a_team_prefix_is_rejected() {
        assert!(apple("noteamprefix", &[]).is_err());
        assert!(apple("", &[]).is_err());
        // A dotted id is accepted — "ABCDE12345.net.example.app" and even
        // "net.example.app" parse, because the first segment is the team.
        assert!(apple("ABCDE12345.net.example.app", &[]).is_ok());
    }
}
