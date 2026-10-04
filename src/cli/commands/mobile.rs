//! `soli mobile build android|ios` and `soli mobile publish`: argument
//! handling and printing over `solilang::mobile`, where the work is.

use std::path::{Path, PathBuf};
use std::process;

use solilang::mobile::build::{self, BuildOptions, IosExport};
use solilang::mobile::publish::{self as publisher, PublishOptions};
use solilang::mobile::{config, Platform};

pub struct BuildArgs<'a> {
    pub platform: &'a str,
    pub folder: &'a str,
    pub fcm: Option<bool>,
    pub build_number: Option<&'a str>,
    pub out: Option<&'a str>,
    pub keystore: Option<&'a str>,
    pub export: &'a str,
    pub team_id: Option<&'a str>,
    pub publish: bool,
    pub notes: Option<&'a str>,
    pub url: Option<&'a str>,
}

fn fail(message: &str) -> ! {
    eprintln!("Error: {message}");
    process::exit(1);
}

fn platform_or_exit(raw: &str) -> Platform {
    Platform::parse(raw).unwrap_or_else(|| {
        eprintln!("Unknown platform '{raw}': expected android or ios");
        process::exit(64);
    })
}

pub fn build(args: BuildArgs<'_>) {
    let platform = platform_or_exit(args.platform);
    let Some(ios_export) = IosExport::parse(args.export) else {
        eprintln!(
            "Unknown --export '{}': expected ad-hoc or enterprise",
            args.export
        );
        process::exit(64);
    };
    if args.fcm.is_some() && platform == Platform::Ios {
        eprintln!("--fcm is for android builds");
        process::exit(64);
    }
    let app_dir = Path::new(args.folder);
    let keystore = args
        .keystore
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("SOLI_ANDROID_KEYSTORE").map(PathBuf::from));
    let out_dir = args.out.map(PathBuf::from);
    let artifact = build::build(&BuildOptions {
        app_dir,
        platform,
        fcm: args.fcm,
        build_number: args.build_number,
        out_dir: out_dir.as_deref(),
        keystore,
        ios_export,
        team_id: args.team_id,
    })
    .unwrap_or_else(|e| fail(&e));

    println!();
    println!(
        "  \x1b[32mBuilt\x1b[0m \x1b[36m{}\x1b[0m",
        artifact.path.display()
    );
    println!(
        "    {} {} (build {}) · {} bytes · sha256 {}",
        artifact.name, artifact.version, artifact.build_number, artifact.size, artifact.sha256
    );
    if args.publish {
        upload(&artifact.path, app_dir, args.notes, args.url);
    }
}

pub fn publish(
    file: Option<&str>,
    latest: Option<&str>,
    folder: &str,
    notes: Option<&str>,
    url: Option<&str>,
) {
    let app_dir = Path::new(folder);
    let file = match (file, latest) {
        (Some(file), _) => PathBuf::from(file),
        (None, Some(platform)) => {
            let platform = platform_or_exit(platform);
            let dist = app_dir.join("dist/mobile");
            build::latest_artifact(&dist, platform).unwrap_or_else(|| {
                fail(&format!(
                    "no .{} in {}: run `soli mobile build {}` first",
                    platform.extension(),
                    dist.display(),
                    platform.as_str()
                ))
            })
        }
        (None, None) => unreachable!("the parser requires a file or --latest"),
    };
    upload(&file, app_dir, notes, url);
}

fn upload(file: &Path, app_dir: &Path, notes: Option<&str>, url: Option<&str>) {
    let config = config::load_or_default(app_dir).unwrap_or_else(|e| fail(&e));
    let url = url
        .map(str::to_string)
        .or(config.publish_url)
        .or(config.url)
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| {
            fail(
                "no publish URL: pass --url, or set `url` under [mobile.publish] in \
                 config/mobile.toml",
            )
        });
    let token = std::env::var("SOLI_MOBILE_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    println!("==> Uploading {} to {url}", file.display());
    let published = publisher::publish(&PublishOptions {
        file,
        url: &url,
        notes,
        token: token.as_deref(),
    })
    .unwrap_or_else(|e| fail(&e));
    println!();
    println!(
        "  \x1b[32mPublished\x1b[0m {} (build {})",
        published.version, published.build_number
    );
    println!("    Install:  \x1b[36m{}\x1b[0m", published.install_url);
    println!("    Download: {}", published.download_url);
}
