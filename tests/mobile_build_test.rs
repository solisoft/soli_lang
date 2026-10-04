//! `soli mobile build`: picking the shell, and stamping the version into the
//! trees `soli generate client` writes.
//!
//! The compile step needs an Android SDK. Without `ANDROID_HOME` it is skipped;
//! with `SOLI_REQUIRE_ANDROID=1` a missing SDK fails the test instead, so a CI
//! job that provisions one cannot pass by skipping.

use std::path::Path;

use solilang::mobile::build::{self, AndroidShell, BuildOptions, IosExport};
use solilang::mobile::{stamp, Platform};
use solilang::scaffold::{create_client, ClientFlags, ClientOptions};

fn generate(app: &Path, platform: &str, fcm: bool) {
    let config = solilang::mobile::config::MobileConfig::default();
    let opts = ClientOptions::resolve(
        platform,
        &app.to_string_lossy(),
        &ClientFlags {
            url: Some("https://staging.shop.test".into()),
            package_id: Some("com.example.shop".into()),
            app_name: Some("Shop".into()),
            fcm: Some(fcm),
            ..Default::default()
        },
        &config,
    );
    create_client(&opts).expect("generate client");
}

#[test]
fn the_shell_is_picked_from_the_flag_or_from_what_exists() {
    let app = tempfile::tempdir().unwrap();
    assert_eq!(
        build::pick_android_shell(app.path(), None).unwrap(),
        AndroidShell::Plain,
        "nothing generated yet: the plain shell"
    );

    generate(app.path(), "android", true);
    assert_eq!(
        build::pick_android_shell(app.path(), None).unwrap(),
        AndroidShell::Fcm,
        "only the FCM shell exists"
    );

    generate(app.path(), "android", false);
    let err = build::pick_android_shell(app.path(), None).unwrap_err();
    assert!(
        err.contains("--fcm"),
        "both exist: refuse and say how to choose: {err}"
    );
    assert_eq!(
        build::pick_android_shell(app.path(), Some(true)).unwrap(),
        AndroidShell::Fcm
    );
    assert_eq!(
        build::pick_android_shell(app.path(), Some(false)).unwrap(),
        AndroidShell::Plain
    );
}

#[test]
fn both_generated_android_trees_take_the_version() {
    let app = tempfile::tempdir().unwrap();
    generate(app.path(), "android", false);
    generate(app.path(), "android", true);

    let manifest =
        std::fs::read_to_string(app.path().join("clients/android/AndroidManifest.xml")).unwrap();
    let stamped = stamp::android_manifest(&manifest, "1.4.0", 42).unwrap();
    assert!(stamped.contains(r#"android:versionName="1.4.0""#));
    assert!(stamped.contains(r#"android:versionCode="42""#));

    let gradle =
        std::fs::read_to_string(app.path().join("clients/android-fcm/app/build.gradle")).unwrap();
    let stamped = stamp::gradle(&gradle, "1.4.0", 42).unwrap();
    assert!(stamped.contains("versionName '1.4.0'"), "{stamped}");
    assert!(stamped.contains("versionCode 42"), "{stamped}");
}

#[test]
fn the_generated_ios_plist_takes_the_version() {
    let app = tempfile::tempdir().unwrap();
    generate(app.path(), "ios", false);
    let target = build::ios_target_dir(&app.path().join("clients/ios")).unwrap();
    let plist = std::fs::read_to_string(target.join("Info.plist")).unwrap();
    let stamped = stamp::info_plist(&plist, "2.0.0", 7).unwrap();
    assert!(stamped.contains("<string>2.0.0</string>"));
    assert!(stamped.contains("<string>7</string>"));
}

#[test]
fn the_staged_copy_leaves_the_developers_tree_alone() {
    let app = tempfile::tempdir().unwrap();
    generate(app.path(), "android", false);
    let source = app.path().join("clients/android");
    let staged = tempfile::tempdir().unwrap();
    build::copy_shell(&source, staged.path()).unwrap();
    let path = staged.path().join("AndroidManifest.xml");
    let stamped =
        stamp::android_manifest(&std::fs::read_to_string(&path).unwrap(), "9.9.9", 99).unwrap();
    std::fs::write(&path, stamped).unwrap();
    let original = std::fs::read_to_string(source.join("AndroidManifest.xml")).unwrap();
    assert!(original.contains(r#"android:versionName="1.0""#));
}

#[test]
fn the_plain_shell_compiles_when_an_sdk_is_present() {
    if std::env::var_os("ANDROID_HOME").is_none() {
        assert!(
            std::env::var("SOLI_REQUIRE_ANDROID").as_deref() != Ok("1"),
            "SOLI_REQUIRE_ANDROID=1 but ANDROID_HOME is not set"
        );
        eprintln!("skipped: no ANDROID_HOME");
        return;
    }
    let app = tempfile::tempdir().unwrap();
    std::fs::write(
        app.path().join("soli.toml"),
        "[package]\nname = \"shop\"\nversion = \"1.4.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(app.path().join("config")).unwrap();
    std::fs::write(
        app.path().join("config/mobile.toml"),
        "[mobile]\nname = \"Shop\"\n",
    )
    .unwrap();
    generate(app.path(), "android", false);
    let artifact = build::build(&BuildOptions {
        app_dir: app.path(),
        platform: Platform::Android,
        fcm: None,
        build_number: Some("42"),
        out_dir: None,
        keystore: None,
        ios_export: IosExport::AdHoc,
        team_id: None,
    })
    .expect("build");
    assert_eq!(artifact.version, "1.4.0");
    assert_eq!(artifact.build_number, "42");
    assert_eq!(artifact.bundle_id, "com.example.shop");
    assert!(artifact.path.ends_with("dist/mobile/shop-1.4.0-42.apk"));
    assert!(build::read_stub(&artifact.path).is_some());
    assert!(
        app.path().join("clients/android/debug.keystore").is_file(),
        "the generated debug key is kept so later builds install as updates"
    );
}
