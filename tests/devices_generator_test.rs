//! `soli generate devices` / `client` / `app_links` smoke tests.

use std::fs;
use std::path::Path;

use solilang::scaffold::{
    create_app_links, create_client, create_devices, create_offline, AppLinksOptions, ClientOptions,
};

fn make_app_skeleton(root: &Path) {
    for sub in [
        "app/controllers",
        "app/models",
        "app/views",
        "app/helpers",
        "config",
    ] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
}

#[test]
fn create_devices_writes_model_controller_and_routes() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    create_devices(dir.path().to_str().unwrap()).expect("generate devices");

    assert!(dir.path().join("app/models/device.sl").exists());
    assert!(dir
        .path()
        .join("app/controllers/devices_controller.sl")
        .exists());
    assert!(dir.path().join("app/helpers/devices_helper.sl").exists());

    let migrations: Vec<_> = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        migrations.iter().any(|n| n.contains("create_devices")),
        "missing create_devices migration: {migrations:?}"
    );

    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert!(routes.contains("devices#create"));
    assert!(routes.contains("devices#destroy"));
    assert!(
        routes.contains("skip_csrf(\"/devices/*\")"),
        "devices routes should skip CSRF for shell token POSTs: {routes}"
    );

    // Idempotent
    create_devices(dir.path().to_str().unwrap()).expect("second run");
}

#[test]
fn create_client_android_writes_manifest() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    let opts = ClientOptions {
        platform: "android".into(),
        url: "https://app.example.com/".into(),
        package_id: "net.example.demo".into(),
        scheme: "demo".into(),
        app_name: "Demo".into(),
        team_id: "TEAM".into(),
        fcm: false,
        folder: dir.path().to_str().unwrap().into(),
    };
    create_client(&opts).expect("generate android client");

    let manifest =
        fs::read_to_string(dir.path().join("clients/android/AndroidManifest.xml")).unwrap();
    assert!(manifest.contains("net.example.demo"));
    assert!(manifest.contains("app.example.com"));
    assert!(manifest.contains("demo"));

    let java = fs::read_to_string(
        dir.path()
            .join("clients/android/src/net/example/demo/MainActivity.java"),
    )
    .unwrap();
    assert!(java.contains("https://app.example.com/"));
}

#[test]
fn create_client_android_fcm_writes_gradle() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    let opts = ClientOptions {
        platform: "android".into(),
        url: "https://app.example.com/".into(),
        package_id: "net.example.demo".into(),
        scheme: "demo".into(),
        app_name: "Demo".into(),
        team_id: "TEAM".into(),
        fcm: true,
        folder: dir.path().to_str().unwrap().into(),
    };
    create_client(&opts).expect("generate android-fcm");

    assert!(dir
        .path()
        .join("clients/android-fcm/app/build.gradle")
        .exists());
    let svc = fs::read_to_string(dir.path().join(
        "clients/android-fcm/app/src/main/java/net/example/demo/SoliFirebaseMessagingService.java",
    ))
    .unwrap();
    assert!(svc.contains("FirebaseMessagingService"));
}

#[test]
fn create_client_ios_writes_swift() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    let opts = ClientOptions {
        platform: "ios".into(),
        url: "https://app.example.com/".into(),
        package_id: "net.example.demo".into(),
        scheme: "demo".into(),
        app_name: "Demo".into(),
        team_id: "ABCDE12345".into(),
        fcm: false,
        folder: dir.path().to_str().unwrap().into(),
    };
    create_client(&opts).expect("generate ios");

    let app_delegate =
        fs::read_to_string(dir.path().join("clients/ios/Demo/AppDelegate.swift")).unwrap();
    assert!(app_delegate.contains("didRegisterForRemoteNotifications"));
    assert!(app_delegate.contains("/devices"));
}

#[test]
fn create_offline_writes_sync_and_outbox() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    create_offline(dir.path().to_str().unwrap()).expect("generate offline");

    assert!(dir.path().join("app/models/sync_event.sl").exists());
    assert!(dir
        .path()
        .join("app/controllers/sync_controller.sl")
        .exists());
    assert!(dir.path().join("public/js/soli_outbox.js").exists());

    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert!(routes.contains("sync#push"));
    assert!(routes.contains("sync#pull"));
    assert!(routes.contains("skip_csrf(\"/sync/*\")"));

    let migration_names: Vec<_> = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(migration_names
        .iter()
        .any(|n| n.contains("create_sync_events")));

    let mig = fs::read_to_string(
        dir.path().join("db/migrations").join(
            migration_names
                .iter()
                .find(|n| n.contains("create_sync_events"))
                .unwrap(),
        ),
    )
    .unwrap();
    assert!(mig.contains("begin"), "migration should use begin/rescue");
}

#[test]
fn create_app_links_writes_well_known_controller() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    let opts = AppLinksOptions {
        android_package: "net.example.demo".into(),
        android_sha256: "AA:BB".into(),
        apple_app_id: "TEAM.net.example.demo".into(),
        paths: vec!["/pings/*".into()],
    };
    create_app_links(dir.path().to_str().unwrap(), &opts).expect("app_links");

    let body =
        fs::read_to_string(dir.path().join("app/controllers/well_known_controller.sl")).unwrap();
    assert!(body.contains("AppLinks.android"));
    assert!(body.contains("AppLinks.apple"));

    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert!(routes.contains("assetlinks.json"));
    assert!(routes.contains("apple-app-site-association"));
}
