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

const EMITTED_FILES: [&str; 18] = [
    "app/services/oidc_config.sl",
    "app/services/oidc_helper.sl",
    "app/models/oauth_client.sl",
    "app/models/oauth_authorization_code.sl",
    "app/models/oauth_refresh_token.sl",
    "app/models/oauth_consent.sl",
    "app/models/oauth_revocation.sl",
    "app/models/oauth_device_code.sl",
    "app/controllers/oidc_discovery_controller.sl",
    "app/controllers/oauth_authorizations_controller.sl",
    "app/controllers/oauth_tokens_controller.sl",
    "app/controllers/oauth_registrations_controller.sl",
    "app/controllers/oauth_device_controller.sl",
    "app/controllers/oauth_userinfo_controller.sl",
    "app/controllers/oauth_sessions_controller.sl",
    "app/views/oauth_authorizations/new.html.slv",
    "app/views/oauth_authorizations/error.html.slv",
    "app/views/oauth_device/new.html.slv",
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
        "\"oauth_tokens#introspect\"",
        "\"oauth_tokens#device_authorization\"",
        "\"oauth_registrations#create\"",
        "\"oauth_device#new\"",
        "skip_csrf(\"/oauth/introspect\")",
        "skip_csrf(\"/oauth/register\")",
        "skip_csrf(\"/oauth/device_authorization\")",
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

/// The guards the audit asked for, as the templates state them: a verified
/// logout hint, introspection of one's own tokens only, refresh tokens bound
/// to the user's session version, no plain-http remote redirect URI through
/// dynamic registration, and `prompt` / `max_age` never silently ignored.
#[test]
fn emitted_templates_carry_the_security_guards() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oidc_provider(dir.path().to_str().unwrap()).unwrap();
    let read = |rel: &str| fs::read_to_string(dir.path().join(rel)).unwrap();

    let sessions = read("app/controllers/oauth_sessions_controller.sl");
    assert!(!sessions.contains("jwt_decode_unsafe("), "{sessions}");
    assert!(sessions.contains("jwt_verify(hint"), "{sessions}");
    assert!(
        sessions.contains(r#""issuer": oidc_issuer()"#),
        "{sessions}"
    );
    assert!(sessions.contains(r#""leeway":"#), "{sessions}");
    // Only option keys `jwt_verify` knows.
    for key in [r#""aud":"#, r#""iss":"#, r#""verify_exp""#] {
        assert!(!sessions.contains(key), "{key} in {sessions}");
    }

    let tokens = read("app/controllers/oauth_tokens_controller.sl");
    assert!(tokens.contains("may_introspect?"), "{tokens}");
    assert!(tokens.contains("session_version_changed"), "{tokens}");
    let client = read("app/models/oauth_client.sl");
    assert!(client.contains("def may_introspect?"), "{client}");
    assert!(client.contains("def registrable_redirect_uri?"), "{client}");
    let refresh = read("app/models/oauth_refresh_token.sl");
    assert!(
        refresh.contains(r#""session_version": session_version"#),
        "{refresh}"
    );

    let registrations = read("app/controllers/oauth_registrations_controller.sl");
    assert!(
        registrations.contains("OauthClient.registrable_redirect_uri?(uri)"),
        "{registrations}"
    );

    let authorizations = read("app/controllers/oauth_authorizations_controller.sl");
    assert!(
        authorizations.contains("login_required"),
        "{authorizations}"
    );
    assert!(
        authorizations.contains("consent_required"),
        "{authorizations}"
    );
    assert!(
        !authorizations.contains(r#"session_get("auth_time") ?? DateTime"#),
        "auth_time must not pretend the sign-in happened now"
    );
}

/// A grant made before the user's session version moved (a password reset,
/// an account takeover) dies with it: the authorization code and the device
/// approval record the version, the token endpoint refuses them once it moved,
/// refresh tokens carry the version of the grant they came from (never the
/// user's current one), and introspection agrees with the token endpoint.
/// Logout redirects only where the client registered, and never straight away
/// for a visitor with no session; `prompt=consent` shows the consent screen.
#[test]
fn grants_are_bound_to_the_session_version_and_logout_is_not_a_redirector() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oidc_provider(dir.path().to_str().unwrap()).unwrap();
    let read = |rel: &str| fs::read_to_string(dir.path().join(rel)).unwrap();

    let authorizations = read("app/controllers/oauth_authorizations_controller.sl");
    assert!(
        authorizations.contains(r#""session_version": user.current_session_version"#),
        "{authorizations}"
    );
    assert!(
        authorizations.contains(r#"!prompts.includes?("consent")"#),
        "{authorizations}"
    );
    let codes = read("app/models/oauth_authorization_code.sl");
    assert!(
        codes.contains(r#""session_version": attributes["session_version"] ?? 0"#),
        "{codes}"
    );
    let device = read("app/controllers/oauth_device_controller.sl");
    assert!(device.contains("user.current_session_version"), "{device}");
    let device_codes = read("app/models/oauth_device_code.sl");
    assert!(
        device_codes.contains("session_version: #{session_version}"),
        "{device_codes}"
    );

    let tokens = read("app/controllers/oauth_tokens_controller.sl");
    assert!(
        tokens
            .matches(r#"_session_version_moved?(user, row["session_version"])"#)
            .count()
            == 2,
        "the code and device grants must both check the version:\n{tokens}"
    );
    assert!(
        tokens.contains(r#"_session_version_moved?(user, token["session_version"])"#),
        "{tokens}"
    );
    assert!(tokens.contains("this._refresh_live?(refresh)"), "{tokens}");
    let issue = tokens
        .split("def _issue_tokens")
        .nth(1)
        .and_then(|rest| rest.split("\n  end").next())
        .unwrap();
    assert!(
        !issue.contains("current_session_version"),
        "refresh tokens must carry the grant's version:\n{issue}"
    );

    let sessions = read("app/controllers/oauth_sessions_controller.sl");
    assert!(
        sessions.contains("post_logout_redirect_uri_allowed?(requested)"),
        "{sessions}"
    );
    assert!(sessions.contains(r#""signed_out": true"#), "{sessions}");
    assert!(
        !sessions.contains("user.nil? || hint"),
        "a hint must not end a session that is not there:\n{sessions}"
    );
    let client = read("app/models/oauth_client.sl");
    assert!(
        client.contains("def post_logout_redirect_uri_allowed?"),
        "{client}"
    );
    let view = read("app/views/oauth/logout_confirm.html.slv");
    assert!(view.contains("signed_out"), "{view}");
    let registrations = read("app/controllers/oauth_registrations_controller.sl");
    assert!(
        registrations.contains(r#"metadata["post_logout_redirect_uris"]"#),
        "{registrations}"
    );
}

/// Dynamic registration takes plain http only to this machine — and the host
/// must really be the loopback one: no backslash, nothing glued after it.
#[test]
fn registrable_redirect_uris_are_loopback_only_over_http() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oidc_provider(dir.path().to_str().unwrap()).unwrap();
    let cases = [
        ("http://localhost:80\\\\evil.example", false),
        ("http://[::1]evil.example/", false),
        ("http://localhost.evil.example/", false),
        ("http://localhost@evil.example/", false),
        ("http://127.0.0.1:99999x/", false),
        ("http://localhost:/", false),
        ("https://app.example/cb\\\\x", false),
        ("http://example.com/cb", false),
        ("http://localhost", true),
        ("http://localhost:8080/cb", true),
        ("http://127.0.0.1:3000/cb?x=1", true),
        ("http://[::1]:5000/", true),
        ("http://LOCALHOST/cb", true),
        ("https://app.example/cb", true),
    ];
    let script = cases
        .iter()
        .map(|(uri, _)| format!("print(OauthClient.registrable_redirect_uri?(\"{uri}\"))"))
        .collect::<Vec<_>>()
        .join("\n");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(["runner", "-e", &script])
        .current_dir(dir.path())
        .env_remove("SOLI_ENGINE")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", dir.path().display()),
        )
        .output()
        .expect("run soli runner");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answers: Vec<&str> = stdout
        .lines()
        .filter(|line| *line == "true" || *line == "false")
        .collect();
    assert_eq!(answers.len(), cases.len(), "{stdout}");
    for ((uri, expected), answer) in cases.iter().zip(answers) {
        assert_eq!(answer, expected.to_string(), "{uri}");
    }
}
