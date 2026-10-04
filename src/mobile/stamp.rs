//! Writing the app's version into a copy of a shell before it is compiled.
//!
//! The generated shells hard-code `1.0` / `1`: the Android manifest's
//! `android:versionName` / `android:versionCode`, the FCM shell's
//! `defaultConfig` in `app/build.gradle`, and the iOS `Info.plist`'s
//! `CFBundleShortVersionString` / `CFBundleVersion`. These functions rewrite
//! those values in place and refuse when one is missing, so a shell the
//! developer has reshaped fails loudly instead of shipping as `1.0`.

use regex::Regex;

/// A version name safe to drop into XML, Gradle and a plist unescaped.
pub fn validate_version_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "version {name:?} must be 1-64 characters of letters, digits, '.', '-', '+' or '_'"
        ))
    }
}

/// Android's `versionCode` ceiling (Play refuses anything above it).
pub const MAX_BUILD_NUMBER: u64 = 2_100_000_000;

/// The build number: the flag, else the CI run number (`GITHUB_RUN_NUMBER`,
/// `CI_PIPELINE_IID`), else minutes since the Unix epoch, which only grows.
pub fn resolve_build_number(
    flag: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
    now_unix_secs: u64,
) -> Result<u64, String> {
    let (raw, source) = match flag {
        Some(value) => (value.to_string(), "--build-number"),
        None => match ["GITHUB_RUN_NUMBER", "CI_PIPELINE_IID"]
            .iter()
            .find_map(|var| env(var).filter(|v| !v.trim().is_empty()).map(|v| (v, *var)))
        {
            Some(found) => found,
            None => return Ok(now_unix_secs / 60),
        },
    };
    match raw.trim().parse::<u64>() {
        Ok(n) if (1..=MAX_BUILD_NUMBER).contains(&n) => Ok(n),
        _ => Err(format!(
            "{source} must be a whole number from 1 to {MAX_BUILD_NUMBER}, got {raw:?}"
        )),
    }
}

/// `android:versionName` / `android:versionCode` on the `<manifest>` element.
pub fn android_manifest(xml: &str, name: &str, code: u64) -> Result<String, String> {
    validate_version_name(name)?;
    let xml = replace_one(
        xml,
        r#"android:versionName\s*=\s*"[^"]*""#,
        &format!(r#"android:versionName="{name}""#),
        "android:versionName in AndroidManifest.xml",
    )?;
    replace_one(
        &xml,
        r#"android:versionCode\s*=\s*"[^"]*""#,
        &format!(r#"android:versionCode="{code}""#),
        "android:versionCode in AndroidManifest.xml",
    )
}

/// `versionName` / `versionCode` in a Groovy `app/build.gradle` (`=` optional).
pub fn gradle(script: &str, name: &str, code: u64) -> Result<String, String> {
    validate_version_name(name)?;
    let script = replace_one(
        script,
        r#"versionName(\s*=\s*|\s+)['"][^'"]*['"]"#,
        &format!("versionName '{name}'"),
        "versionName in app/build.gradle",
    )?;
    replace_one(
        &script,
        r#"versionCode(\s*=\s*|\s+)\d+"#,
        &format!("versionCode {code}"),
        "versionCode in app/build.gradle",
    )
}

/// `CFBundleShortVersionString` / `CFBundleVersion` in an `Info.plist`.
pub fn info_plist(plist: &str, name: &str, code: u64) -> Result<String, String> {
    validate_version_name(name)?;
    let plist = replace_one(
        plist,
        r#"(<key>CFBundleShortVersionString</key>\s*)<string>[^<]*</string>"#,
        &format!("${{1}}<string>{name}</string>"),
        "CFBundleShortVersionString in Info.plist",
    )?;
    replace_one(
        &plist,
        r#"(<key>CFBundleVersion</key>\s*)<string>[^<]*</string>"#,
        &format!("${{1}}<string>{code}</string>"),
        "CFBundleVersion in Info.plist",
    )
}

fn replace_one(text: &str, pattern: &str, with: &str, what: &str) -> Result<String, String> {
    let re = Regex::new(pattern).expect("stamp pattern");
    match re.find_iter(text).count() {
        1 => Ok(re.replace(text, with).into_owned()),
        0 => Err(format!("cannot stamp the version: no {what}")),
        _ => Err(format!(
            "cannot stamp the version: {what} appears more than once"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_attributes_are_rewritten() {
        let xml = "<manifest package=\"a.b\"\n    android:versionCode=\"1\"\n    android:versionName=\"1.0\">";
        let out = android_manifest(xml, "1.4.0", 42).unwrap();
        assert!(out.contains(r#"android:versionCode="42""#), "{out}");
        assert!(out.contains(r#"android:versionName="1.4.0""#), "{out}");
    }

    #[test]
    fn gradle_lines_are_rewritten_with_or_without_equals() {
        let script = "defaultConfig {\n  versionCode 1\n  versionName '1.0'\n}";
        let out = gradle(script, "2.0.1", 7).unwrap();
        assert!(out.contains("versionCode 7") && out.contains("versionName '2.0.1'"));
        let assigned = "versionCode = 3\nversionName = \"0.1\"";
        let out = gradle(assigned, "0.2", 4).unwrap();
        assert!(out.contains("versionCode 4") && out.contains("versionName '0.2'"));
    }

    #[test]
    fn plist_strings_are_rewritten() {
        let plist = "<key>CFBundleShortVersionString</key>\n  <string>1.0</string>\n  <key>CFBundleVersion</key>\n  <string>1</string>";
        let out = info_plist(plist, "3.1", 99).unwrap();
        assert!(out.contains("<key>CFBundleShortVersionString</key>\n  <string>3.1</string>"));
        assert!(out.contains("<key>CFBundleVersion</key>\n  <string>99</string>"));
    }

    #[test]
    fn a_missing_field_is_refused() {
        let err = android_manifest("<manifest android:versionCode=\"1\">", "1.0", 1).unwrap_err();
        assert!(err.contains("versionName"), "{err}");
    }

    #[test]
    fn an_unsafe_version_is_refused() {
        assert!(validate_version_name("1.0\"><x").is_err());
        assert!(validate_version_name("").is_err());
        assert!(validate_version_name("1.4.0-beta+3").is_ok());
    }

    #[test]
    fn build_number_prefers_flag_then_ci_then_time() {
        let none = |_: &str| None;
        assert_eq!(resolve_build_number(Some("42"), none, 0).unwrap(), 42);
        let github = |v: &str| (v == "GITHUB_RUN_NUMBER").then(|| "17".to_string());
        assert_eq!(resolve_build_number(None, github, 0).unwrap(), 17);
        let gitlab = |v: &str| (v == "CI_PIPELINE_IID").then(|| "5".to_string());
        assert_eq!(resolve_build_number(None, gitlab, 0).unwrap(), 5);
        assert_eq!(resolve_build_number(None, none, 6000).unwrap(), 100);
        assert!(resolve_build_number(Some("abc"), none, 0).is_err());
        assert!(resolve_build_number(Some("0"), none, 0).is_err());
        assert!(resolve_build_number(Some("2100000001"), none, 0).is_err());
    }
}
