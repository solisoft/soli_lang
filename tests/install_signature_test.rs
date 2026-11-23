//! `install.sh` and the release signature.
//!
//! The installer is run for real — `sh install.sh --user`, in a scratch
//! `HOME` — against a local release host, with the two GitHub URLs of the
//! script pointed at it. The release is signed here as the pipeline signs it
//! (`update::release_statement`, the key `soli update-keygen` makes) and
//! verified by the script with `openssl`, so the test also holds the two
//! implementations to the same bytes.

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;

use base64::Engine;
use p256::ecdsa::signature::Signer;
use p256::pkcs8::DecodePrivateKey;
use sha2::{Digest, Sha256};

const TAG: &str = "v9.9.9";

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A release whose `soli` is a script that says its version.
fn tarball() -> Vec<u8> {
    let script = b"#!/bin/sh\necho \"soli 9.9.9\"\n";
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder
        .append_data(&mut header, "soli", &script[..])
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

/// The artifact name the script asks for on this machine.
fn artifact() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("soli-{os}-{arch}.tar.gz")
}

fn sign(private_pem: &str, tag: &str, artifact: &str, tarball: &[u8]) -> String {
    let key = p256::ecdsa::SigningKey::from_pkcs8_pem(private_pem).unwrap();
    let statement = solilang::update::release_statement(tag, artifact, &sha256_hex(tarball));
    let signature: p256::ecdsa::Signature = key.sign(&statement);
    base64::engine::general_purpose::STANDARD.encode(signature.to_der().as_bytes())
}

/// Serve the release: the API's "latest", the tarball, its digest and — when
/// there is one — its signature.
fn release_host(tarball: Vec<u8>, signature: Option<String>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let sha = sha256_hex(&tarball);
    std::thread::spawn(move || {
        for _ in 0..8 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let path = String::from_utf8_lossy(&buf[..n])
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            let (status, body): (&str, Vec<u8>) = if path.ends_with("/latest") {
                (
                    "200 OK",
                    format!("{{\n  \"tag_name\": \"{TAG}\"\n}}\n").into_bytes(),
                )
            } else if path.ends_with(".sha256") {
                ("200 OK", sha.clone().into_bytes())
            } else if path.ends_with(".sig") {
                match &signature {
                    Some(sig) => ("200 OK", sig.clone().into_bytes()),
                    None => ("404 Not Found", Vec::new()),
                }
            } else if path.ends_with(".tar.gz") {
                ("200 OK", tarball.clone())
            } else {
                ("404 Not Found", Vec::new())
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    port
}

struct Install {
    output: String,
    installed: bool,
}

/// Run the installer against a release host. `pubkey` is what the release
/// key line of the script would hold.
fn install(signature: Option<String>, pubkey: Option<&str>) -> Install {
    let port = release_host(tarball(), signature);
    let dir = tempfile::tempdir().unwrap();
    let script =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh")).unwrap();
    let api = "https://api.github.com/repos/${REPO}/releases/latest";
    let downloads = "https://github.com/${REPO}/releases/download";
    assert!(script.contains(api) && script.contains(downloads));
    let script = script
        .replace(api, &format!("http://127.0.0.1:{port}/api/latest"))
        .replace(downloads, &format!("http://127.0.0.1:{port}/dl"));
    let script_path = dir.path().join("install.sh");
    std::fs::write(&script_path, script).unwrap();

    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut command = Command::new("sh");
    command
        .arg(&script_path)
        .arg("--user")
        .env("HOME", &home)
        .env_remove("SOLI_RELEASE_PUBKEY")
        .env_remove("SOLI_INSTALL_SKIP_CHECKSUM");
    if let Some(key) = pubkey {
        command.env("SOLI_RELEASE_PUBKEY", key);
    }
    let out = command.output().expect("sh should run");
    Install {
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        installed: home.join(".local/bin/soli").exists(),
    }
}

#[test]
fn the_installer_installs_only_what_the_release_key_signed() {
    assert!(
        Command::new("openssl").arg("version").output().is_ok(),
        "this test needs openssl, as the installer does once a release key exists"
    );
    let (private_pem, public) = solilang::update::generate_keypair().unwrap();
    let bytes = tarball();
    let name = artifact();

    // Signed for this tag and this artifact.
    let run = install(Some(sign(&private_pem, TAG, &name, &bytes)), Some(&public));
    assert!(run.output.contains("Signature verified."), "{}", run.output);
    assert!(run.installed, "{}", run.output);

    // The same tarball signed as an older release: a rollback.
    let run = install(
        Some(sign(&private_pem, "v9.9.8", &name, &bytes)),
        Some(&public),
    );
    assert!(
        run.output.contains("is not the release key's for"),
        "{}",
        run.output
    );
    assert!(!run.installed);

    // Signed as another artifact.
    let run = install(
        Some(sign(
            &private_pem,
            TAG,
            "render-pdf-linux-amd64.tar.gz",
            &bytes,
        )),
        Some(&public),
    );
    assert!(
        run.output.contains("is not the release key's for"),
        "{}",
        run.output
    );
    assert!(!run.installed);

    // Signed by another key.
    let (stranger_pem, _) = solilang::update::generate_keypair().unwrap();
    let run = install(Some(sign(&stranger_pem, TAG, &name, &bytes)), Some(&public));
    assert!(
        run.output.contains("is not the release key's for"),
        "{}",
        run.output
    );
    assert!(!run.installed);

    // No signature published.
    let run = install(None, Some(&public));
    assert!(
        run.output.contains("publishes no signature"),
        "{}",
        run.output
    );
    assert!(!run.installed);
}

/// Until the key is written into the script, the digest is what is checked —
/// and the script in the repository must not carry a key by accident of an
/// environment: this is the shipped behaviour today.
#[test]
fn without_a_release_key_the_installer_checks_the_digest() {
    let run = install(None, None);
    assert!(run.output.contains("Checksum verified."), "{}", run.output);
    assert!(!run.output.contains("Signature"), "{}", run.output);
    assert!(run.installed, "{}", run.output);
}

/// "Latest" is what a server said: a release older than the installed one is
/// not installed, signed or not, unless the person asks for it.
#[test]
fn the_installer_does_not_go_back_to_an_older_release() {
    let port = release_host(tarball(), None);
    let dir = tempfile::tempdir().unwrap();
    let script = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh"))
        .unwrap()
        .replace(
            "https://api.github.com/repos/${REPO}/releases/latest",
            &format!("http://127.0.0.1:{port}/api/latest"),
        )
        .replace(
            "https://github.com/${REPO}/releases/download",
            &format!("http://127.0.0.1:{port}/dl"),
        );
    let script_path = dir.path().join("install.sh");
    std::fs::write(&script_path, script).unwrap();
    let bin = dir.path().join("home/.local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let installed = bin.join("soli");
    std::fs::write(&installed, "#!/bin/sh\necho \"soli 10.0.0\"\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let out = Command::new("sh")
        .arg(&script_path)
        .arg("--user")
        .env("HOME", dir.path().join("home"))
        .env_remove("SOLI_RELEASE_PUBKEY")
        .env_remove("SOLI_INSTALL_ALLOW_DOWNGRADE")
        .output()
        .unwrap();
    let output = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        output.contains("is older than the installed soli 10.0.0"),
        "{output}"
    );
    assert!(std::fs::read_to_string(&installed)
        .unwrap()
        .contains("10.0.0"));
}
