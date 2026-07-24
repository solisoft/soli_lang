//! `soli generate client <platform>` — native shell projects.

use std::fs;
use std::path::{Path, PathBuf};

use crate::scaffold::app_generator::write_file;
use crate::scaffold::templates::clients::ClientCtx;
use crate::scaffold::templates::clients::{android, android_fcm, ios, linux, windows};

#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub platform: String,
    pub url: String,
    pub package_id: String,
    pub scheme: String,
    pub app_name: String,
    pub team_id: String,
    pub fcm: bool,
    pub folder: String,
}

impl ClientOptions {
    pub fn host(&self) -> String {
        url_host(&self.url).unwrap_or_else(|| "localhost".to_string())
    }
}

fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split('/').next()?.split('@').next_back()?;
    let host = host.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Generate a native shell under `clients/<platform>/` (or `clients/android-fcm/`).
pub fn create_client(opts: &ClientOptions) -> Result<(), String> {
    let app_path = Path::new(&opts.folder);
    if !app_path.exists() {
        return Err(format!("Directory '{}' does not exist", opts.folder));
    }

    let platform = opts.platform.to_lowercase();
    let ctx = ClientCtx {
        app_name: opts.app_name.clone(),
        start_url: normalize_url(&opts.url),
        host: opts.host(),
        package_id: opts.package_id.clone(),
        scheme: opts.scheme.clone(),
        team_id: opts.team_id.clone(),
    };

    match platform.as_str() {
        "android" if opts.fcm => write_android_fcm(app_path, &ctx)?,
        "android" => write_android(app_path, &ctx)?,
        "ios" => write_ios(app_path, &ctx)?,
        "linux" => write_linux(app_path, &ctx)?,
        "windows" | "win" => write_windows(app_path, &ctx)?,
        "macos" | "mac" => {
            return Err(
                "macOS local/desktop products use `soli desktop build` (or embed with SOLI_DESKTOP_NO_WINDOW). \
                 For a remote WebView shell, use `soli generate client ios` as a starting point — \
                 a dedicated macos-remote template is not generated yet."
                    .to_string(),
            );
        }
        other => {
            return Err(format!(
                "Unknown client platform '{other}'. Try: android, ios, linux, windows (android --fcm for FCM)."
            ));
        }
    }

    Ok(())
}

fn normalize_url(url: &str) -> String {
    let mut u = url.trim().to_string();
    if !u.ends_with('/') {
        u.push('/');
    }
    u
}

fn write_file_ctx(path: &Path, template: &str, ctx: &ClientCtx) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
    }
    if path.exists() {
        println!(
            "  \x1b[33mskip\x1b[0m   {} (already exists)",
            path.display()
        );
        return Ok(());
    }
    let body = ctx.apply(template);
    write_file(path, &body)?;
    println!("  \x1b[32mcreate\x1b[0m {}", path.display());
    Ok(())
}

fn write_android(app_path: &Path, ctx: &ClientCtx) -> Result<(), String> {
    let root = app_path.join("clients/android");
    write_file_ctx(&root.join("README.txt"), android::README, ctx)?;
    write_file_ctx(&root.join("AndroidManifest.xml"), android::MANIFEST, ctx)?;
    write_file_ctx(&root.join("build.sh"), android::BUILD_SH, ctx)?;
    // mark executable bit best-effort
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(root.join("build.sh"), fs::Permissions::from_mode(0o755));
    }
    write_file_ctx(&root.join("res/values/strings.xml"), android::STRINGS, ctx)?;
    write_file_ctx(&root.join("res/values/styles.xml"), android::STYLES, ctx)?;
    write_file_ctx(&root.join("res/values/colors.xml"), android::COLORS, ctx)?;
    let java = root.join(format!(
        "src/{}/MainActivity.java",
        ctx.package_id.replace('.', "/")
    ));
    write_file_ctx(&java, android::MAIN_ACTIVITY, ctx)?;
    // Placeholder launcher: 1x1 PNG is invalid for aapt; document that user must add icons.
    write_placeholder_icon_note(&root)?;
    Ok(())
}

fn write_placeholder_icon_note(root: &Path) -> Result<(), String> {
    let note = root.join("res/mipmap-mdpi/README.txt");
    if note.exists() {
        return Ok(());
    }
    if let Some(parent) = note.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
    }
    write_file(
        &note,
        "Add ic_launcher.png here (and hdpi/xhdpi/xxhdpi/xxxhdpi). \
         The generated manifest omits android:icon so aapt2 works without assets.\n",
    )?;
    println!("  \x1b[32mcreate\x1b[0m {}", note.display());
    Ok(())
}

fn write_android_fcm(app_path: &Path, ctx: &ClientCtx) -> Result<(), String> {
    let root = app_path.join("clients/android-fcm");
    write_file_ctx(&root.join("README.txt"), android_fcm::README, ctx)?;
    write_file_ctx(
        &root.join("settings.gradle"),
        android_fcm::SETTINGS_GRADLE,
        ctx,
    )?;
    write_file_ctx(
        &root.join("build.gradle"),
        android_fcm::ROOT_BUILD_GRADLE,
        ctx,
    )?;
    write_file_ctx(
        &root.join("gradle.properties"),
        android_fcm::GRADLE_PROPERTIES,
        ctx,
    )?;
    write_file_ctx(
        &root.join("app/build.gradle"),
        android_fcm::APP_BUILD_GRADLE,
        ctx,
    )?;
    write_file_ctx(
        &root.join("app/src/main/AndroidManifest.xml"),
        android_fcm::MANIFEST,
        ctx,
    )?;
    let java_dir = format!("app/src/main/java/{}", ctx.package_id.replace('.', "/"));
    write_file_ctx(
        &root.join(format!("{java_dir}/MainActivity.java")),
        android_fcm::MAIN_ACTIVITY,
        ctx,
    )?;
    write_file_ctx(
        &root.join(format!("{java_dir}/SoliFirebaseMessagingService.java")),
        android_fcm::FCM_SERVICE,
        ctx,
    )?;
    write_file_ctx(
        &root.join("app/google-services.json.example"),
        android_fcm::GOOGLE_SERVICES_PLACEHOLDER,
        ctx,
    )?;
    Ok(())
}

fn write_ios(app_path: &Path, ctx: &ClientCtx) -> Result<(), String> {
    let root = app_path.join("clients/ios");
    let app_dir = root.join(&ctx.app_name);
    write_file_ctx(&root.join("README.txt"), ios::README, ctx)?;
    write_file_ctx(&root.join("project.yml"), ios::PROJECT_YML, ctx)?;
    write_file_ctx(&app_dir.join("Info.plist"), ios::INFO_PLIST, ctx)?;
    write_file_ctx(&app_dir.join("App.entitlements"), ios::ENTITLEMENTS, ctx)?;
    write_file_ctx(&app_dir.join("AppDelegate.swift"), ios::APP_DELEGATE, ctx)?;
    write_file_ctx(
        &app_dir.join("WebViewController.swift"),
        ios::WEB_VIEW_CONTROLLER,
        ctx,
    )?;
    Ok(())
}

fn write_linux(app_path: &Path, ctx: &ClientCtx) -> Result<(), String> {
    let root = app_path.join("clients/linux");
    write_file_ctx(&root.join("README.txt"), linux::README, ctx)?;
    write_file_ctx(&root.join("Cargo.toml"), linux::CARGO_TOML, ctx)?;
    write_file_ctx(&root.join("src/main.rs"), linux::MAIN_RS, ctx)?;
    write_file_ctx(
        &root.join(format!("{}.desktop", ctx.scheme)),
        linux::DESKTOP_FILE,
        ctx,
    )?;
    write_file_ctx(&root.join("install.sh"), linux::INSTALL_SH, ctx)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(root.join("install.sh"), fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

fn write_windows(app_path: &Path, ctx: &ClientCtx) -> Result<(), String> {
    let root = app_path.join("clients/windows");
    let proj = root.join(format!("{}Shell", ctx.scheme));
    write_file_ctx(&root.join("README.txt"), windows::README, ctx)?;
    write_file_ctx(
        &proj.join(format!("{}Shell.csproj", ctx.scheme)),
        windows::CSPROJ,
        ctx,
    )?;
    write_file_ctx(&proj.join("Program.cs"), windows::PROGRAM_CS, ctx)?;
    write_file_ctx(
        &root.join("register-protocol.ps1"),
        windows::REGISTER_PS1,
        ctx,
    )?;
    Ok(())
}

pub fn print_client_success_message(opts: &ClientOptions) {
    let dir = match (opts.platform.to_lowercase().as_str(), opts.fcm) {
        ("android", true) => "clients/android-fcm",
        ("android", false) => "clients/android",
        ("ios", _) => "clients/ios",
        ("linux", _) => "clients/linux",
        ("windows" | "win", _) => "clients/windows",
        _ => "clients/",
    };
    println!();
    println!("  \x1b[32mClient scaffold ready:\x1b[0m \x1b[36m{dir}\x1b[0m");
    println!("  See the README in that directory for build steps.");
    println!("  Register devices: \x1b[36msoli generate devices\x1b[0m");
    println!("  Deep-link proofs:  \x1b[36msoli generate app_links\x1b[0m");
    println!();
}

/// Resolve default package / name from folder when flags omitted.
pub fn defaults_from_folder(folder: &str) -> (String, String) {
    let name = PathBuf::from(folder)
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .filter(|s| s != "." && !s.is_empty())
        .unwrap_or_else(|| "SoliApp".to_string());
    let scheme = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    let scheme = if scheme.is_empty() {
        "myapp".to_string()
    } else {
        scheme
    };
    let package = format!("net.example.{}", scheme);
    (name, package)
}
