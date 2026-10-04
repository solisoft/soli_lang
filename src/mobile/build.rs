//! `soli mobile build android|ios`: from `clients/<platform>/` to an
//! installable, versioned APK or IPA under `dist/mobile/`.
//!
//! The shell is copied to a temporary directory and stamped there with the
//! app's version (`[package].version` in `soli.toml`) and a build number, so
//! the developer's tree is never modified by a build. Two things do land in
//! it, both on purpose: a missing shell is generated into `clients/` the way
//! `soli generate client` would, and the plain Android shell's
//! `debug.keystore` is kept there once `build.sh` has created it. A new key
//! per build would make Android refuse every update over an installed copy.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::config::MobileConfig;
use super::{stamp, Platform};

/// The two Android shells `soli generate client` can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidShell {
    /// `clients/android/`: `build.sh` (aapt2, javac, d8, apksigner), no Gradle.
    Plain,
    /// `clients/android-fcm/`: a Gradle project with Firebase Cloud Messaging.
    Fcm,
}

impl AndroidShell {
    pub fn dir(self) -> &'static str {
        match self {
            AndroidShell::Plain => "clients/android",
            AndroidShell::Fcm => "clients/android-fcm",
        }
    }
}

/// Which Android shell to build: `--fcm` / `fcm = true` (or `false`) decides;
/// otherwise whichever of the two exists. Both existing with no say is refused
/// rather than guessed.
pub fn pick_android_shell(app_dir: &Path, fcm: Option<bool>) -> Result<AndroidShell, String> {
    if let Some(fcm) = fcm {
        return Ok(if fcm {
            AndroidShell::Fcm
        } else {
            AndroidShell::Plain
        });
    }
    let plain = app_dir.join(AndroidShell::Plain.dir()).is_dir();
    let fcm = app_dir.join(AndroidShell::Fcm.dir()).is_dir();
    match (plain, fcm) {
        (true, true) => Err(format!(
            "both {} and {} exist: pass --fcm to build the FCM shell, or set \
             `fcm = true` / `fcm = false` in config/mobile.toml",
            AndroidShell::Plain.dir(),
            AndroidShell::Fcm.dir()
        )),
        (false, true) => Ok(AndroidShell::Fcm),
        _ => Ok(AndroidShell::Plain),
    }
}

/// How an IPA is exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IosExport {
    /// Ad hoc: devices registered on the team's provisioning profile.
    AdHoc,
    /// In-house distribution, for an Apple Developer Enterprise account.
    Enterprise,
}

impl IosExport {
    pub fn parse(raw: &str) -> Option<IosExport> {
        match raw {
            "ad-hoc" | "adhoc" | "release-testing" => Some(IosExport::AdHoc),
            "enterprise" => Some(IosExport::Enterprise),
            _ => None,
        }
    }
}

pub struct BuildOptions<'a> {
    pub app_dir: &'a Path,
    pub platform: Platform,
    /// `--fcm`; `None` defers to the config, then to which shell exists.
    pub fcm: Option<bool>,
    pub build_number: Option<&'a str>,
    /// `--out`; default `<app>/dist/mobile`.
    pub out_dir: Option<&'a Path>,
    /// `--keystore` / `SOLI_ANDROID_KEYSTORE`: a release keystore. Its
    /// passwords are read from the environment, never from argv.
    pub keystore: Option<PathBuf>,
    pub ios_export: IosExport,
    /// `--team-id`, over `team_id` in the config.
    pub team_id: Option<&'a str>,
}

/// What a build produced, as written to the `.mobile.json` stub.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Artifact {
    pub platform: String,
    pub name: String,
    pub bundle_id: String,
    pub version: String,
    pub build_number: String,
    pub sha256: String,
    pub size: u64,
    #[serde(skip)]
    pub path: PathBuf,
}

/// The `.mobile.json` stub written next to an artifact.
pub fn stub_path(artifact: &Path) -> PathBuf {
    let mut p = artifact.as_os_str().to_os_string();
    p.push(".mobile.json");
    PathBuf::from(p)
}

/// Read the stub next to `artifact`, if there is one.
pub fn read_stub(artifact: &Path) -> Option<Artifact> {
    let text = std::fs::read_to_string(stub_path(artifact)).ok()?;
    let mut parsed: Artifact = serde_json::from_str(&text).ok()?;
    parsed.path = artifact.to_path_buf();
    Some(parsed)
}

/// `[package].version` from `soli.toml`, or `0.0.0` when there is none.
pub fn app_version(app_dir: &Path) -> Result<String, String> {
    let manifest = app_dir.join("soli.toml");
    if !manifest.is_file() {
        return Ok("0.0.0".to_string());
    }
    let package = crate::module::Package::load(&manifest)
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
    Ok(if package.version.is_empty() {
        "0.0.0".to_string()
    } else {
        package.version
    })
}

/// A file-name-safe slug: `My Shop!` → `my-shop`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "app".to_string()
    } else {
        out
    }
}

/// Copy `src` into `dst`, leaving out build output that a compile recreates.
pub fn copy_shell(src: &Path, dst: &Path) -> Result<(), String> {
    const SKIP: [&str; 5] = ["build", ".gradle", "DerivedData", "app.apk", ".DS_Store"];
    for entry in walkdir::WalkDir::new(src).into_iter().filter_entry(|e| {
        e.depth() == 0 || !SKIP.contains(&e.file_name().to_string_lossy().as_ref())
    }) {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", src.display()))?;
        let rel = entry
            .path()
            .strip_prefix(src)
            .expect("walkdir stays under src");
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)
                .map_err(|e| format!("cannot create {}: {e}", target.display()))?;
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), &target)
                .map_err(|e| format!("cannot copy {}: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// The shell to build, generating it into `clients/` first when it is missing.
fn ensure_shell(
    app_dir: &Path,
    platform: Platform,
    shell: Option<AndroidShell>,
    config: &MobileConfig,
) -> Result<PathBuf, String> {
    let rel = match (platform, shell) {
        (Platform::Android, Some(shell)) => shell.dir(),
        (Platform::Android, None) => AndroidShell::Plain.dir(),
        (Platform::Ios, _) => "clients/ios",
    };
    let dir = app_dir.join(rel);
    if dir.is_dir() {
        return Ok(dir);
    }
    if shell == Some(AndroidShell::Fcm) {
        return Err(format!(
            "{rel} does not exist: run `soli generate client android --fcm`, then add \
             app/google-services.json from your Firebase project"
        ));
    }
    if config.url.as_deref().unwrap_or("").is_empty() {
        return Err(format!(
            "{rel} does not exist, and config/mobile.toml has no `url` to generate it from: \
             run `soli generate client {} --url https://…` or add `url` under [mobile]",
            platform.as_str()
        ));
    }
    let folder = app_dir.to_string_lossy().to_string();
    let opts = crate::scaffold::ClientOptions::resolve(
        platform.as_str(),
        &folder,
        &crate::scaffold::ClientFlags {
            fcm: Some(false),
            ..Default::default()
        },
        config,
    );
    println!("==> Generating {rel}");
    crate::scaffold::create_client(&opts)?;
    Ok(dir)
}

/// Build one artifact. Prints the tool chains' output as it goes.
pub fn build(opts: &BuildOptions<'_>) -> Result<Artifact, String> {
    let config = super::config::load_or_default(opts.app_dir)?;
    let version = app_version(opts.app_dir)?;
    stamp::validate_version_name(&version)
        .map_err(|e| format!("[package].version in soli.toml: {e}"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let build_number =
        stamp::resolve_build_number(opts.build_number, |v| std::env::var(v).ok(), now)?;

    let shell = match opts.platform {
        Platform::Android => Some(pick_android_shell(opts.app_dir, opts.fcm.or(config.fcm))?),
        Platform::Ios => None,
    };
    // The tool chain is checked before a missing shell is generated, so a
    // build that cannot run leaves nothing behind in `clients/`.
    match (opts.platform, shell) {
        (Platform::Ios, _) => preflight_ios()?,
        (Platform::Android, Some(AndroidShell::Plain)) => preflight_android_plain()?,
        _ => {}
    }
    let source = ensure_shell(opts.app_dir, opts.platform, shell, &config)?;

    let staging = tempfile::Builder::new()
        .prefix("soli-mobile-")
        .tempdir()
        .map_err(|e| format!("cannot create a temporary directory: {e}"))?;
    let work = staging.path().join("shell");
    copy_shell(&source, &work)?;

    let (built, bundle_id) = match (opts.platform, shell) {
        (Platform::Android, Some(AndroidShell::Fcm)) => {
            stamp_file(&work.join("app/build.gradle"), |text| {
                stamp::gradle(text, &version, build_number)
            })?;
            let bundle_id = read_attr(
                &work.join("app/build.gradle"),
                r#"applicationId\s*=?\s*['"]([^'"]+)['"]"#,
            );
            (
                build_android_fcm(&work, opts.keystore.as_deref())?,
                bundle_id,
            )
        }
        (Platform::Android, _) => {
            stamp_file(&work.join("AndroidManifest.xml"), |text| {
                stamp::android_manifest(text, &version, build_number)
            })?;
            let bundle_id = read_attr(
                &work.join("AndroidManifest.xml"),
                r#"<manifest[^>]*\spackage\s*=\s*"([^"]+)""#,
            );
            let apk = build_android_plain(&work, opts.keystore.as_deref())?;
            keep_debug_keystore(&source, &work);
            (apk, bundle_id)
        }
        (Platform::Ios, _) => {
            let team_id = opts
                .team_id
                .map(str::to_string)
                .or_else(|| config.team_id.clone())
                .filter(|t| !t.is_empty() && t != "TEAMID")
                .ok_or(
                    "an iOS build needs your Apple team id: pass --team-id or set `team_id` \
                     in config/mobile.toml",
                )?;
            let target = ios_target_dir(&work)?;
            stamp_file(&target.join("Info.plist"), |text| {
                stamp::info_plist(text, &version, build_number)
            })?;
            let bundle_id = read_attr(
                &work.join("project.yml"),
                r"PRODUCT_BUNDLE_IDENTIFIER:\s*([A-Za-z0-9.\-]+)",
            );
            (
                build_ios(&work, &target, &team_id, opts.ios_export)?,
                bundle_id,
            )
        }
    };

    let name = config
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            crate::scaffold::client_generator::defaults_from_folder(&opts.app_dir.to_string_lossy())
                .0
        });
    let out_dir = opts
        .out_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| opts.app_dir.join("dist/mobile"));
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
    let file = out_dir.join(format!(
        "{}-{version}-{build_number}.{}",
        slug(&name),
        opts.platform.extension()
    ));
    std::fs::copy(&built, &file).map_err(|e| format!("cannot write {}: {e}", file.display()))?;

    let artifact = Artifact {
        platform: opts.platform.as_str().to_string(),
        name,
        bundle_id: bundle_id.unwrap_or_default(),
        version,
        build_number: build_number.to_string(),
        sha256: super::sha256_file(&file).map_err(|e| e.to_string())?,
        size: std::fs::metadata(&file).map_err(|e| e.to_string())?.len(),
        path: file.clone(),
    };
    let stub = serde_json::to_string_pretty(&artifact).map_err(|e| e.to_string())?;
    std::fs::write(stub_path(&file), stub).map_err(|e| e.to_string())?;
    Ok(artifact)
}

fn stamp_file(path: &Path, f: impl Fn(&str) -> Result<String, String>) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let stamped = f(&text)?;
    std::fs::write(path, stamped).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn read_attr(path: &Path, pattern: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    regex::Regex::new(pattern)
        .ok()?
        .captures(&text)?
        .get(1)
        .map(|m| m.as_str().to_string())
}

fn on_path(tool: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(tool).is_file())
}

fn run(step: &str, command: &mut Command) -> Result<(), String> {
    println!("==> {step}");
    let status = command
        .status()
        .map_err(|e| format!("{step}: cannot start {:?}: {e}", command.get_program()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{step} failed ({status})"))
    }
}

/// Everything `build.sh` needs, named up front instead of failing halfway.
pub fn android_sdk_problems(
    android_home: Option<&Path>,
    has_tool: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut missing = Vec::new();
    match android_home {
        None => missing.push("ANDROID_HOME is not set (point it at your Android SDK)".to_string()),
        Some(home) => {
            let build_tools = home.join("build-tools/35.0.0");
            if !build_tools.join("aapt2").is_file() {
                missing.push(format!(
                    "build-tools 35.0.0 ({}): sdkmanager \"build-tools;35.0.0\"",
                    build_tools.display()
                ));
            }
            let platform = home.join("platforms/android-34/android.jar");
            if !platform.is_file() {
                missing.push(format!(
                    "platform 34 ({}): sdkmanager \"platforms;android-34\"",
                    platform.display()
                ));
            }
        }
    }
    for tool in ["javac", "jar", "keytool", "zip"] {
        if !has_tool(tool) {
            missing.push(format!("`{tool}` on PATH (a JDK 17+, and zip)"));
        }
    }
    missing
}

fn preflight_android_plain() -> Result<(), String> {
    let home = std::env::var_os("ANDROID_HOME").map(PathBuf::from);
    let missing = android_sdk_problems(home.as_deref(), on_path);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the Android build cannot run; missing:\n  - {}",
            missing.join("\n  - ")
        ))
    }
}

fn build_android_plain(work: &Path, keystore: Option<&Path>) -> Result<PathBuf, String> {
    let home = std::env::var_os("ANDROID_HOME").map(PathBuf::from);
    run(
        "Compiling the Android shell (build.sh)",
        Command::new("bash").arg("build.sh").current_dir(work),
    )?;
    let apk = work.join("app.apk");
    let Some(keystore) = keystore else {
        return Ok(apk);
    };
    // `build.sh` signed with the debug key; sign the aligned APK again with
    // the release key. Passwords come from the environment, never from argv.
    let password_env = "SOLI_ANDROID_KEYSTORE_PASSWORD";
    if std::env::var_os(password_env).is_none() {
        return Err(format!(
            "--keystore needs {password_env} in the environment"
        ));
    }
    let apksigner = home
        .ok_or("ANDROID_HOME is not set")?
        .join("build-tools/35.0.0/apksigner");
    let release = work.join("app-release.apk");
    let mut sign = Command::new(apksigner);
    sign.arg("sign")
        .arg("--ks")
        .arg(keystore)
        .arg("--ks-pass")
        .arg(format!("env:{password_env}"));
    if let Ok(alias) = std::env::var("SOLI_ANDROID_KEY_ALIAS") {
        sign.arg("--ks-key-alias").arg(alias);
    }
    if std::env::var_os("SOLI_ANDROID_KEY_PASSWORD").is_some() {
        sign.arg("--key-pass").arg("env:SOLI_ANDROID_KEY_PASSWORD");
    }
    sign.arg("--out")
        .arg(&release)
        .arg(work.join("build/aligned.apk"));
    run("Signing with the release keystore", &mut sign)?;
    Ok(release)
}

/// Keep the key `build.sh` generated, so the next build signs with it too.
fn keep_debug_keystore(source: &Path, work: &Path) {
    let kept = source.join("debug.keystore");
    let made = work.join("debug.keystore");
    if !kept.exists() && made.is_file() && std::fs::copy(&made, &kept).is_ok() {
        println!("    Kept {} for the next builds", kept.display());
    }
}

fn build_android_fcm(work: &Path, keystore: Option<&Path>) -> Result<PathBuf, String> {
    if !work.join("app/google-services.json").is_file() {
        return Err(
            "app/google-services.json is missing from the FCM shell: download it from your \
             Firebase project (Project settings → Your apps) and save it as \
             clients/android-fcm/app/google-services.json"
                .to_string(),
        );
    }
    let wrapper = work.join("gradlew");
    let mut gradle = if wrapper.is_file() {
        let mut c = Command::new("sh");
        c.arg(wrapper);
        c
    } else if on_path("gradle") {
        Command::new("gradle")
    } else {
        return Err(
            "the FCM shell needs Gradle: install it (`gradle` on PATH), or add a wrapper \
             with `gradle wrapper` in clients/android-fcm"
                .to_string(),
        );
    };
    gradle.current_dir(work);
    let (task, output) = match keystore {
        // An unsigned release APK does not install, so without a keystore the
        // debug variant (signed with the machine's debug key) is built.
        None => (
            ":app:assembleDebug",
            "app/build/outputs/apk/debug/app-debug.apk",
        ),
        Some(keystore) => {
            let password = std::env::var("SOLI_ANDROID_KEYSTORE_PASSWORD").map_err(|_| {
                "--keystore needs SOLI_ANDROID_KEYSTORE_PASSWORD in the environment".to_string()
            })?;
            let alias = std::env::var("SOLI_ANDROID_KEY_ALIAS").map_err(|_| {
                "--keystore on the FCM shell needs SOLI_ANDROID_KEY_ALIAS in the environment"
                    .to_string()
            })?;
            let key_password =
                std::env::var("SOLI_ANDROID_KEY_PASSWORD").unwrap_or_else(|_| password.clone());
            let keystore = std::fs::canonicalize(keystore)
                .map_err(|e| format!("keystore {}: {e}", keystore.display()))?;
            // Project properties through ORG_GRADLE_PROJECT_*, not -P on argv,
            // so no password shows in `ps`.
            for (property, value) in [
                ("store.file", keystore.to_string_lossy().to_string()),
                ("store.password", password),
                ("key.alias", alias),
                ("key.password", key_password),
            ] {
                gradle.env(
                    format!("ORG_GRADLE_PROJECT_android.injected.signing.{property}"),
                    value,
                );
            }
            (
                ":app:assembleRelease",
                "app/build/outputs/apk/release/app-release.apk",
            )
        }
    };
    gradle.arg(task);
    run(
        &format!("Compiling the FCM shell (gradle {task})"),
        &mut gradle,
    )?;
    Ok(work.join(output))
}

fn preflight_ios() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("an iOS build needs macOS with Xcode".to_string());
    }
    let mut missing = Vec::new();
    if !on_path("xcodegen") {
        missing.push("`xcodegen` (brew install xcodegen)");
    }
    if !on_path("xcodebuild") {
        missing.push("`xcodebuild` (install Xcode, then xcode-select --install)");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the iOS build cannot run; missing:\n  - {}",
            missing.join("\n  - ")
        ))
    }
}

/// The app target's folder: the one directory holding an `Info.plist`.
pub fn ios_target_dir(work: &Path) -> Result<PathBuf, String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(work)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        if entry.path().join("Info.plist").is_file() {
            found.push(entry.path());
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err("clients/ios has no <App>/Info.plist".to_string()),
        _ => Err("clients/ios has more than one <App>/Info.plist".to_string()),
    }
}

/// `xcodebuild -exportArchive`'s options. Xcode 15.3 renamed `ad-hoc` to
/// `release-testing`; older Xcodes only know the old name.
pub fn export_options_plist(
    export: IosExport,
    team_id: &str,
    xcode_major_minor: (u32, u32),
) -> String {
    let method = match export {
        IosExport::Enterprise => "enterprise",
        IosExport::AdHoc if xcode_major_minor >= (15, 3) => "release-testing",
        IosExport::AdHoc => "ad-hoc",
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key>
  <string>{method}</string>
  <key>signingStyle</key>
  <string>automatic</string>
  <key>teamID</key>
  <string>{team}</string>
</dict>
</plist>
"#,
        team = super::xml_escape(team_id)
    )
}

fn xcode_version() -> (u32, u32) {
    let out = Command::new("xcodebuild").arg("-version").output();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    // "Xcode 16.2\nBuild version 16C5032a"
    let version = text
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("Xcode "))
        .unwrap_or("0.0");
    let mut parts = version.split('.').map(|p| p.trim().parse().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

fn build_ios(
    work: &Path,
    target: &Path,
    team_id: &str,
    export: IosExport,
) -> Result<PathBuf, String> {
    let app = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    run(
        "Generating the Xcode project (xcodegen)",
        Command::new("xcodegen").arg("generate").current_dir(work),
    )?;
    let archive = work.join(format!("build/{app}.xcarchive"));
    run(
        "Archiving (xcodebuild archive)",
        Command::new("xcodebuild")
            .current_dir(work)
            .arg("-project")
            .arg(format!("{app}.xcodeproj"))
            .arg("-scheme")
            .arg(&app)
            .arg("-configuration")
            .arg("Release")
            .arg("-destination")
            .arg("generic/platform=iOS")
            .arg("-archivePath")
            .arg(&archive)
            .arg("-allowProvisioningUpdates")
            .arg("archive")
            .arg(format!("DEVELOPMENT_TEAM={team_id}"))
            .arg("CODE_SIGN_STYLE=Automatic"),
    )?;
    let options = work.join("build/ExportOptions.plist");
    std::fs::write(
        &options,
        export_options_plist(export, team_id, xcode_version()),
    )
    .map_err(|e| e.to_string())?;
    let export_dir = work.join("build/export");
    run(
        "Exporting the IPA (xcodebuild -exportArchive)",
        Command::new("xcodebuild")
            .current_dir(work)
            .arg("-exportArchive")
            .arg("-archivePath")
            .arg(&archive)
            .arg("-exportPath")
            .arg(&export_dir)
            .arg("-exportOptionsPlist")
            .arg(&options)
            .arg("-allowProvisioningUpdates"),
    )?;
    std::fs::read_dir(&export_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "ipa"))
        .ok_or_else(|| format!("xcodebuild exported no .ipa into {}", export_dir.display()))
}

/// The newest artifact for `platform` in `dir`, for `publish --latest`.
pub fn latest_artifact(dir: &Path, platform: Platform) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| Platform::from_file_name(&p.to_string_lossy()) == Some(platform))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_file_name_safe() {
        assert_eq!(slug("My Shop!"), "my-shop");
        assert_eq!(slug("shop"), "shop");
        assert_eq!(slug("***"), "app");
    }

    #[test]
    fn sdk_problems_name_each_missing_piece() {
        let none = android_sdk_problems(None, |_| false);
        assert!(none[0].contains("ANDROID_HOME"));
        assert!(none.iter().any(|p| p.contains("javac")));

        let sdk = tempfile::tempdir().unwrap();
        let problems = android_sdk_problems(Some(sdk.path()), |_| true);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("build-tools;35.0.0"));
        assert!(problems[1].contains("platforms;android-34"));

        std::fs::create_dir_all(sdk.path().join("build-tools/35.0.0")).unwrap();
        std::fs::write(sdk.path().join("build-tools/35.0.0/aapt2"), "").unwrap();
        std::fs::create_dir_all(sdk.path().join("platforms/android-34")).unwrap();
        std::fs::write(sdk.path().join("platforms/android-34/android.jar"), "").unwrap();
        assert!(android_sdk_problems(Some(sdk.path()), |_| true).is_empty());
    }

    #[test]
    fn export_method_follows_the_xcode_version() {
        assert!(export_options_plist(IosExport::AdHoc, "T", (16, 0)).contains("release-testing"));
        assert!(export_options_plist(IosExport::AdHoc, "T", (15, 2)).contains(">ad-hoc<"));
        assert!(export_options_plist(IosExport::Enterprise, "T", (16, 0)).contains("enterprise"));
    }

    #[test]
    fn copy_shell_leaves_build_output_behind() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("build/x")).unwrap();
        std::fs::write(src.path().join("build/x/junk"), "").unwrap();
        std::fs::write(src.path().join("app.apk"), "").unwrap();
        std::fs::create_dir_all(src.path().join("res/values")).unwrap();
        std::fs::write(src.path().join("res/values/strings.xml"), "s").unwrap();
        let dst = tempfile::tempdir().unwrap();
        copy_shell(src.path(), dst.path()).unwrap();
        assert!(dst.path().join("res/values/strings.xml").is_file());
        assert!(!dst.path().join("build").exists());
        assert!(!dst.path().join("app.apk").exists());
    }
}
