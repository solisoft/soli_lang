//! `config/mobile.toml`: what `soli generate client`, `soli mobile build` and
//! `soli mobile publish` share, so they stop taking the same flags three times.
//!
//! ```toml
//! [mobile]
//! url = "https://staging.shop.example.com"
//! package = "com.example.shop"
//! scheme = "shop"
//! name = "Shop"
//! team_id = "ABCDE12345"
//! fcm = false
//!
//! [mobile.publish]
//! url = "https://staging.shop.example.com"
//! ```
//!
//! Not a section of `soli.toml`: `Package::parse` refuses any section but
//! `[package]` and `[dependencies]`, so a `[mobile]` there would make every
//! older soli refuse the project. For the same reason in reverse, an unknown
//! key here is a warning and not an error: a key added later must not break
//! the first releases that read this file.
//!
//! The file is optional, and every value in it is a default a CLI flag
//! overrides. The app's version is not repeated here — it is `[package].version`
//! in `soli.toml`.

use std::path::Path;

pub const CONFIG_PATH: &str = "config/mobile.toml";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MobileConfig {
    pub url: Option<String>,
    pub package: Option<String>,
    pub scheme: Option<String>,
    pub name: Option<String>,
    pub team_id: Option<String>,
    pub fcm: Option<bool>,
    /// `[mobile.publish] url`: where `soli mobile publish` uploads.
    pub publish_url: Option<String>,
}

/// A parsed file and the keys in it this soli does not know.
#[derive(Debug, Default)]
pub struct Loaded {
    pub config: MobileConfig,
    pub warnings: Vec<String>,
}

/// `config/mobile.toml` under `app_dir`, or `None` when there is none.
pub fn load(app_dir: &Path) -> Result<Option<Loaded>, String> {
    let path = app_dir.join(CONFIG_PATH);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    parse(&content)
        .map(Some)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// [`load`], printing the unknown-key warnings, for the CLI.
pub fn load_or_default(app_dir: &Path) -> Result<MobileConfig, String> {
    Ok(match load(app_dir)? {
        Some(loaded) => {
            for warning in &loaded.warnings {
                eprintln!("warning: {CONFIG_PATH}: {warning}");
            }
            loaded.config
        }
        None => MobileConfig::default(),
    })
}

pub fn parse(content: &str) -> Result<Loaded, String> {
    let root: toml::Table = content
        .parse()
        .map_err(|e: toml::de::Error| e.to_string())?;
    let mut loaded = Loaded::default();
    for (key, value) in &root {
        if key != "mobile" {
            loaded.warnings.push(format!("unknown key `{key}` ignored"));
            continue;
        }
        let toml::Value::Table(mobile) = value else {
            return Err("`mobile` must be a table ([mobile])".to_string());
        };
        for (key, value) in mobile {
            let config = &mut loaded.config;
            match key.as_str() {
                "url" => config.url = Some(string(value, "mobile.url")?),
                "package" => config.package = Some(string(value, "mobile.package")?),
                "scheme" => config.scheme = Some(string(value, "mobile.scheme")?),
                "name" => config.name = Some(string(value, "mobile.name")?),
                "team_id" => config.team_id = Some(string(value, "mobile.team_id")?),
                "fcm" => {
                    config.fcm = Some(
                        value
                            .as_bool()
                            .ok_or("`mobile.fcm` must be true or false")?,
                    )
                }
                "publish" => {
                    let toml::Value::Table(publish) = value else {
                        return Err("`mobile.publish` must be a table ([mobile.publish])".into());
                    };
                    for (key, value) in publish {
                        match key.as_str() {
                            "url" => {
                                config.publish_url = Some(string(value, "mobile.publish.url")?)
                            }
                            other => loaded
                                .warnings
                                .push(format!("unknown key `mobile.publish.{other}` ignored")),
                        }
                    }
                }
                other => loaded
                    .warnings
                    .push(format!("unknown key `mobile.{other}` ignored")),
            }
        }
    }
    Ok(loaded)
}

fn string(value: &toml::Value, name: &str) -> Result<String, String> {
    value
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("`{name}` must be a string"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_key() {
        let loaded = parse(
            r#"
[mobile]
url = "https://staging.shop.example.com"
package = "com.example.shop"
scheme = "shop"
name = "Shop"
team_id = "ABCDE12345"
fcm = true

[mobile.publish]
url = "https://staging.shop.example.com/"
"#,
        )
        .unwrap();
        assert!(loaded.warnings.is_empty());
        assert_eq!(
            loaded.config,
            MobileConfig {
                url: Some("https://staging.shop.example.com".into()),
                package: Some("com.example.shop".into()),
                scheme: Some("shop".into()),
                name: Some("Shop".into()),
                team_id: Some("ABCDE12345".into()),
                fcm: Some(true),
                publish_url: Some("https://staging.shop.example.com/".into()),
            }
        );
    }

    #[test]
    fn unknown_keys_warn_instead_of_failing() {
        let loaded = parse(
            "[mobile]\nurl = \"https://a.test\"\nicon = \"x.png\"\n[mobile.publish]\nchannel = \"beta\"\n[other]\nx = 1\n",
        )
        .unwrap();
        assert_eq!(loaded.config.url.as_deref(), Some("https://a.test"));
        assert_eq!(loaded.warnings.len(), 3, "{:?}", loaded.warnings);
    }

    #[test]
    fn a_wrong_type_is_an_error() {
        assert!(parse("[mobile]\nfcm = \"yes\"\n").is_err());
        assert!(parse("[mobile]\nurl = 3\n").is_err());
        assert!(parse("mobile = 1\n").is_err());
    }

    #[test]
    fn a_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).unwrap().is_none());
        assert_eq!(
            load_or_default(dir.path()).unwrap(),
            MobileConfig::default()
        );
    }
}
