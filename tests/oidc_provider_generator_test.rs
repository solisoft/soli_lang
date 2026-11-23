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

    // Console / seed registration applies the dynamic-registration rule.
    assert!(
        client.contains(
            "redirect_uris.filter(fn(uri) { !OauthClient.registrable_redirect_uri?(uri) })"
        ),
        "{client}"
    );
    // `profile` does not release the address: that is the `email` scope's.
    let config = read("app/services/oidc_config.sl");
    assert!(
        !config.contains(r#"claims["preferred_username"] = user["email"]"#),
        "{config}"
    );
    assert!(
        config.contains(r#"throw "SOLI_OIDC_ISSUER is not set"#),
        "{config}"
    );

    let registrations = read("app/controllers/oauth_registrations_controller.sl");
    assert!(
        registrations.contains("OauthClient.registrable_redirect_uri?(uri)"),
        "{registrations}"
    );

    let authorizations = read("app/controllers/oauth_authorizations_controller.sl");
    // The consent POST holds the sign-in to max_age again.
    assert_eq!(
        authorizations
            .matches("this._stale_login_error(context)")
            .count(),
        2,
        "{authorizations}"
    );
    assert!(
        authorizations.contains(r#""max_age": effective["max_age"].to_s"#),
        "{authorizations}"
    );
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

/// The fourth audit's fixes, read from what the generator writes: a logout
/// forgets the remember-me token; a device user code names one request and a
/// poll writes two fields, not the row; a revoked refresh-token family stays
/// revoked for a token minted meanwhile; an access token dies with the
/// session version and the family it was granted under; an impersonating
/// admin grants nothing; the machine endpoints are throttled.
#[test]
fn the_fourth_audit_s_guards_are_in_the_generated_files() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oidc_provider(dir.path().to_str().unwrap()).unwrap();
    let read = |rel: &str| fs::read_to_string(dir.path().join(rel)).unwrap();
    let has = |source: &str, needle: &str| {
        assert!(source.contains(needle), "missing `{needle}` in:\n{source}")
    };

    let sessions = read("app/controllers/oauth_sessions_controller.sl");
    has(&sessions, "current_user()&.forget_me");
    has(&sessions, r#"set_cookie("remember_token", "", {"#);

    let device = read("app/models/oauth_device_code.sl");
    has(
        &device,
        "FILTER d._key == #{key} AND d.status == \"pending\"",
    );
    has(
        &device,
        "UPDATE d WITH { interval: #{slower}, last_polled_at: #{now} }",
    );
    has(&device, "UPDATE d WITH { last_polled_at: #{now} }");
    assert!(
        !device.contains("\n      row.update()") && !device.contains("\n    row.update()"),
        "a poll writes the whole row:\n{device}"
    );
    has(&device, "throw \"could not issue a device code\"");
    let migration = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| fs::read_to_string(e.path()).unwrap())
        .find(|source| source.contains("idx_oauth_device_user_code"))
        .expect("no device-code migration");
    let index = migration
        .split("idx_oauth_device_user_code")
        .nth(1)
        .unwrap_or_default();
    assert!(
        index
            .split(')')
            .next()
            .unwrap_or_default()
            .contains(r#""unique": true"#),
        "the user-code index is not unique:\n{migration}"
    );

    let refresh = read("app/models/oauth_refresh_token.sl");
    has(&refresh, "OauthRevocation.revoke_family(chain_id, reason)");
    has(&refresh, "if OauthRevocation.family_revoked?(chain_id)");
    let marker = refresh
        .find("OauthRevocation.revoke_family(chain_id, reason)")
        .unwrap();
    let rows = refresh
        .find(r#"tokens = OauthRefreshToken.where({"chain_id": chain_id.to_s})"#)
        .unwrap();
    assert!(
        marker < rows,
        "the family marker must be written before the rows are read"
    );
    let revocation = read("app/models/oauth_revocation.sl");
    has(&revocation, "OIDC_REFRESH_TOKEN_TTL");

    let helper = read("app/services/oidc_helper.sl");
    has(&helper, r#"OauthRevocation.family_revoked?(claims["cid"])"#);
    has(&helper, "oidc_access_token_user_live?(claims)");
    has(
        &helper,
        "user.current_session_version <= claims[\"sv\"].to_i",
    );
    has(&helper, "return false unless user.deactivated_at.nil?");
    let tokens = read("app/controllers/oauth_tokens_controller.sl");
    has(&tokens, r#""sv": (session_version ?? 0).to_i"#);
    has(&tokens, r#""cid": chain_id.to_s"#);
    assert_eq!(
        tokens.matches("this._refused(").count(),
        4,
        "token, revoke, device_authorization and introspect are guarded:\n{tokens}"
    );
    // The guard: the budget, and no credential in the URL.
    has(&tokens, "oidc_within_rate_limit(req, bucket, limit)");
    has(&tokens, "must be sent in the request body, not the URL");
    // Whose code or token it is comes before anything is consumed or revoked.
    let peek = tokens
        .find("OauthAuthorizationCode.peek(code)")
        .expect("no peek");
    let burn = tokens
        .find("OauthAuthorizationCode.burn(code)")
        .expect("no burn");
    assert!(
        peek < burn,
        "the client is checked after the code is burned"
    );
    let owner = tokens
        .find("Refresh token belongs to a different client")
        .unwrap();
    let reuse = tokens.find("if token.reused?()").unwrap();
    assert!(
        owner < reuse,
        "another client can revoke the family by replaying"
    );
    has(&tokens, r#"client.supports_grant?("refresh_token")"#);
    has(&tokens, "client.public?() || client.require_pkce == true");
    has(
        &tokens,
        r#"OauthRevocation.revoke(claims["jti"], "client_revoked")"#,
    );
    has(&refresh, "OIDC_REFRESH_CHAIN_TTL");
    has(&refresh, r#""chain_expires_at": ceiling.to_i"#);
    let config = read("app/services/oidc_config.sl");
    has(&config, "token.length >= 32");
    has(&config, r#"getenv("APP_BASE_URL")"#);
    has(&helper, "&iss=#{url_encode(oidc_issuer())}");
    has(&helper, "oidc_previous_public_key()");

    let authorize = read("app/controllers/oauth_authorizations_controller.sl");
    has(
        &authorize,
        "client.public?() || client.require_pkce == true",
    );
    has(&authorize, "This client must send a signed request object.");
    // The consent POST of a signed request reads what the GET validated.
    has(&authorize, r#""oidc_consent""#);
    has(&authorize, "effective = this._pending_consent(client)");
    has(&tokens, "The sign-in this grant came from has ended");
    has(
        &authorize,
        "The request object expires too far in the future",
    );
    has(
        &authorize,
        r#""session_expires_at": session_get("sign_in_expires_at")"#,
    );
    has(&authorize, "&iss=#{url_encode(oidc_issuer())}");
    assert_eq!(
        authorize
            .matches("return this._impersonating() if this._impersonating?()")
            .count(),
        2,
        "{authorize}"
    );
    let device_controller = read("app/controllers/oauth_device_controller.sl");
    has(
        &device_controller,
        r#"session_get("impersonator_id").to_s.present?"#,
    );
    has(
        &device_controller,
        r#"oidc_within_rate_limit(req, "user_code""#,
    );
    // ... on both ways a user code is looked up: typed in the form (POST)
    // and carried by the link (GET). The link's lookup says whether a code
    // is pending and for which app, so it was the place to guess them.
    assert_eq!(
        device_controller
            .matches(r#"oidc_within_rate_limit(req, "user_code", OIDC_USER_CODE_ATTEMPTS_PER_IP)"#)
            .count(),
        2,
        "{device_controller}"
    );
    let link_lookup = device_controller
        .split("def create")
        .next()
        .unwrap_or_default();
    let throttle = link_lookup
        .find("oidc_within_rate_limit")
        .expect("no throttle in `new`");
    let lookup = link_lookup
        .find("this._confirm_step(code)")
        .expect("no lookup in `new`");
    assert!(
        throttle < lookup,
        "the link's lookup runs before its throttle"
    );
}

/// "Revoked" is answered for what is revoked. A revocation the database did
/// not store — `create` hands the record back unsaved, with its errors —
/// used to be reported as made, over an access token that went on working.
#[test]
fn a_revocation_that_was_not_stored_is_not_answered_as_made() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oidc_provider(dir.path().to_str().unwrap()).unwrap();
    let code = r##"refused = OauthRevocation.new
refused._errors = [{"message": "the database said no"}]
outcome = "answered"
try
  OauthRevocation.must_be_stored(refused, "jti-never-stored")
catch error
  outcome = "raised"
end
print("not stored: #{outcome}")
print("stored: #{OauthRevocation.revoke("jti-1", "test")} #{OauthRevocation.revoked?("jti-1")}")
# Refused because another request wrote the same row first: it is there.
late = OauthRevocation.new
late._errors = [{"message": "duplicate"}]
OauthRevocation.must_be_stored(late, "jti-1")
print("already there: fine")
print("family: #{OauthRevocation.revoke_family("chain-1", "test")} #{OauthRevocation.family_revoked?("chain-1")}")"##;
    let database = format!("sqlite://{}/app.sqlite3", dir.path().display());
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(["runner", "-e", code])
        .current_dir(dir.path())
        .env_remove("SOLI_ENGINE")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env("DATABASE_URL", &database)
        .output()
        .expect("run soli");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    for line in [
        "not stored: raised",
        "stored: true true",
        "already there: fine",
        "family: true true",
    ] {
        assert!(
            text.lines().any(|got| got == line),
            "no `{line}` in\n{text}"
        );
    }
}
