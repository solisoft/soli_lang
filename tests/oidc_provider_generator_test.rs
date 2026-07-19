//! `soli generate oidc_provider` integration tests — exercise the OIDC
//! scaffold generator in a tempdir so `scaffold/oidc_generator.rs` and the
//! embedded templates get covered without polluting the workspace.

use std::fs;
use std::path::Path;

use solilang::scaffold::{create_auth, create_oidc_provider};

/// A skeleton that already carries the auth scaffold, which the OIDC provider
/// builds on.
fn make_app_with_auth(root: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
    create_auth(root.to_str().unwrap()).unwrap();
}

const EMITTED_FILES: [&str; 14] = [
    "app/services/oidc_config.sl",
    "app/services/oidc_helper.sl",
    "app/models/oauth_client.sl",
    "app/models/oauth_authorization_code.sl",
    "app/models/oauth_refresh_token.sl",
    "app/models/oauth_consent.sl",
    "app/models/oauth_revocation.sl",
    "app/controllers/oidc_discovery_controller.sl",
    "app/controllers/oauth_authorizations_controller.sl",
    "app/controllers/oauth_tokens_controller.sl",
    "app/controllers/oauth_userinfo_controller.sl",
    "app/controllers/oauth_sessions_controller.sl",
    "app/views/oauth_authorizations/new.html.slv",
    "app/views/oauth_authorizations/error.html.slv",
];

#[test]
fn create_oidc_provider_writes_all_files() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());

    let result = create_oidc_provider(dir.path().to_str().unwrap());
    assert!(
        result.is_ok(),
        "generate oidc_provider failed: {:?}",
        result.err()
    );

    for rel in EMITTED_FILES {
        assert!(dir.path().join(rel).exists(), "{rel} not created");
    }

    let migrations: Vec<String> = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        migrations.iter().any(|n| n.contains("add_oauth_indexes")),
        "no add_oauth_indexes migration generated: {migrations:?}"
    );
}

/// The migration runner parses `<numeric-version>_<name>.sl`; anything else is
/// skipped *silently*, which would leave the unique indexes — the backstop for
/// single-use codes — quietly absent.
#[test]
fn migrations_use_the_runner_filename_convention() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oidc_provider(dir.path().to_str().unwrap()).unwrap();

    for entry in fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".sl") {
            continue;
        }
        let stem = name.trim_end_matches(".sl");
        let (version, rest) = stem.split_once('_').expect("migration name needs a `_`");
        assert!(
            !version.is_empty() && version.chars().all(|c| c.is_ascii_digit()),
            "migration version must be numeric, got `{version}` in {name}"
        );
        assert!(!rest.is_empty(), "migration {name} has no name part");
    }
}

#[test]
fn create_oidc_provider_appends_routes() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oidc_provider(dir.path().to_str().unwrap()).unwrap();

    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    for expected in [
        "\"oidc_discovery#openid_configuration\"",
        "\"oidc_discovery#jwks\"",
        "\"oauth_authorizations#new\"",
        "\"oauth_tokens#create\"",
        "\"oauth_userinfo#show\"",
    ] {
        assert!(routes.contains(expected), "routes missing {expected}");
    }

    // The token endpoint is called server-to-server, so the same-origin CSRF
    // gate has to be lifted or every legitimate exchange is rejected. This is
    // the single most security-load-bearing line in the snippet, in both
    // directions — assert it explicitly.
    assert!(
        routes.contains("skip_csrf(\"/oauth/token\")"),
        "routes must skip CSRF on the token endpoint"
    );
    assert!(
        !routes.contains("skip_csrf(\"/oauth/authorize\")"),
        "the consent POST is a browser form and must keep CSRF protection"
    );
}

#[test]
fn create_oidc_provider_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oidc_provider(dir.path().to_str().unwrap()).unwrap();

    let routes_before = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    let config_before = fs::read_to_string(dir.path().join("app/services/oidc_config.sl")).unwrap();
    let migrations_before = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .count();

    // A customized file must survive a re-run.
    fs::write(
        dir.path().join("app/services/oidc_config.sl"),
        "# customized\n",
    )
    .unwrap();

    create_oidc_provider(dir.path().to_str().unwrap()).unwrap();

    let routes_after = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert_eq!(routes_before, routes_after, "routes appended twice");
    assert_eq!(
        fs::read_to_string(dir.path().join("app/services/oidc_config.sl")).unwrap(),
        "# customized\n",
        "re-running clobbered a customized file"
    );
    assert_ne!(config_before, "# customized\n");
    assert_eq!(
        fs::read_dir(dir.path().join("db/migrations"))
            .unwrap()
            .count(),
        migrations_before,
        "migration written twice"
    );
}

#[test]
fn create_oidc_provider_requires_the_auth_scaffold() {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["app/controllers", "app/models", "config"] {
        fs::create_dir_all(dir.path().join(sub)).unwrap();
    }

    let err = create_oidc_provider(dir.path().to_str().unwrap())
        .expect_err("must refuse to generate without a User model");
    assert!(
        err.contains("soli generate auth"),
        "the error should name the command to run: {err}"
    );
    assert!(
        !dir.path().join("app/models/oauth_client.sl").exists(),
        "nothing should be written when the precondition fails"
    );
}

#[test]
fn create_oidc_provider_rejects_non_app_directory() {
    let dir = tempfile::tempdir().unwrap();
    let err = create_oidc_provider(dir.path().to_str().unwrap())
        .expect_err("must refuse a directory with no app/");
    assert!(err.contains("does not look like a Soli app"), "{err}");
}

/// Parse every emitted `.sl` file. The templates are strings the compiler never
/// sees, so without this a syntax error ships and only surfaces in a user's
/// app — which is exactly how the block-`unless` and bracket-assignment bugs
/// got in.
#[test]
fn emitted_templates_parse() {
    use solilang::lexer::Scanner;
    use solilang::parser::Parser;

    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oidc_provider(dir.path().to_str().unwrap()).unwrap();

    let mut sources: Vec<(String, String)> = EMITTED_FILES
        .iter()
        .filter(|rel| rel.ends_with(".sl"))
        .map(|rel| {
            (
                rel.to_string(),
                fs::read_to_string(dir.path().join(rel)).unwrap(),
            )
        })
        .collect();

    for entry in fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "sl") {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            sources.push((name, fs::read_to_string(&path).unwrap()));
        }
    }

    assert!(
        sources.len() >= 13,
        "expected the .sl templates + migration"
    );

    for (name, source) in sources {
        let tokens = Scanner::new(&source)
            .scan_tokens()
            .unwrap_or_else(|e| panic!("{name} does not tokenize: {e:?}"));
        let parsed = Parser::new(tokens).parse();
        assert!(parsed.is_ok(), "{name} does not parse: {:?}", parsed.err());
    }
}
