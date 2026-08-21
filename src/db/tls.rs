//! TLS for the Postgres and MySQL clients.
//!
//! Both drivers used to connect in cleartext, which ruled out every managed
//! database (RDS, Cloud SQL, Neon, PlanetScale, Aiven all require an encrypted
//! connection). The stack is **rustls with the `ring` provider** — the same
//! provider the POP3/SMTP/HTTP clients use, so the binary links one crypto
//! backend and still cross-compiles without a system OpenSSL.
//!
//! The mode ladder is libpq's, and the important part of libpq's semantics is
//! kept: **encryption and identity are separate rungs.** `require` encrypts but
//! does not check who is on the other end; verification starts at `verify-ca`.
//! A URL that says `?sslmode=require` therefore behaves exactly as the same URL
//! does under `psql`, and a self-signed server keeps working.
//!
//! | Mode | Encrypts | Verifies chain | Verifies hostname |
//! |------|----------|----------------|-------------------|
//! | `disable` | no | — | — |
//! | `prefer` (default) | if the server offers it | no | no |
//! | `require` | yes | no | no |
//! | `verify-ca` | yes | yes | no |
//! | `verify-full` | yes | yes | yes |
//!
//! MySQL's own spellings (`DISABLED`, `PREFERRED`, `REQUIRED`, `VERIFY_CA`,
//! `VERIFY_IDENTITY`) parse to the same rungs, so a URL copied from either
//! ecosystem's console works unchanged.
//!
//! Neither driver's URL parser tolerates options it does not know
//! (`tokio_postgres` rejects `sslrootcert`, `mysql` rejects anything unlisted),
//! so [`split_url`] lifts the TLS options out of the URL before the driver's
//! parser ever sees it.

use std::path::PathBuf;

/// How far to go in securing a SQL connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SslMode {
    /// Cleartext. What both drivers did before TLS existed here.
    Disable,
    /// Encrypt when the server offers it, unverified. libpq's default, and ours.
    #[default]
    Prefer,
    /// Encryption is mandatory; the certificate is not checked.
    Require,
    /// Encryption is mandatory and the chain must reach a trusted root.
    VerifyCa,
    /// `verify-ca`, plus the certificate must name the host connected to.
    VerifyFull,
}

impl SslMode {
    /// Parse a mode written in either the libpq or the MySQL vocabulary.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let normalized = raw.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "disable" | "disabled" => Ok(Self::Disable),
            "prefer" | "preferred" => Ok(Self::Prefer),
            "require" | "required" => Ok(Self::Require),
            "verify-ca" => Ok(Self::VerifyCa),
            "verify-full" | "verify-identity" => Ok(Self::VerifyFull),
            other => Err(format!(
                "unknown TLS mode {other:?} — use disable, prefer, require, verify-ca, or verify-full"
            )),
        }
    }

    /// Whether the connection is encrypted at all.
    pub fn encrypts(self) -> bool {
        !matches!(self, Self::Disable)
    }

    /// Whether the server's certificate chain is checked against a root.
    pub fn verifies(self) -> bool {
        matches!(self, Self::VerifyCa | Self::VerifyFull)
    }

    /// Whether a failed handshake may fall back to cleartext.
    ///
    /// Only `prefer` may: every stronger rung is a promise the app made.
    pub fn may_fall_back(self) -> bool {
        matches!(self, Self::Prefer)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }
}

/// The TLS options carried by a connection URL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SslConfig {
    /// `None` when the URL said nothing, so a driver that parses `sslmode`
    /// itself keeps its own answer instead of being overridden with a default.
    pub mode: Option<SslMode>,
    /// A CA bundle to trust instead of the built-in Mozilla roots.
    pub root_cert: Option<PathBuf>,
}

impl SslConfig {
    /// The mode to act on, defaulting the way libpq does.
    pub fn mode(&self) -> SslMode {
        self.mode.unwrap_or_default()
    }
}

/// Lift the TLS options out of a connection URL.
///
/// Returns the URL with those parameters removed — safe to hand to the driver's
/// own parser, which rejects options it does not recognise — plus the config
/// they described. A URL with no query string is returned untouched.
pub fn split_url(url: &str) -> Result<(String, SslConfig), String> {
    let Some((head, query)) = url.split_once('?') else {
        return Ok((url.to_string(), SslConfig::default()));
    };
    let mut config = SslConfig::default();
    let mut kept: Vec<&str> = Vec::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "sslmode" | "ssl-mode" => config.mode = Some(SslMode::parse(&decode(value))?),
            "sslrootcert" | "ssl-root-cert" | "ssl-ca" => {
                let path = decode(value);
                if !path.is_empty() {
                    config.root_cert = Some(PathBuf::from(path));
                }
            }
            _ => kept.push(pair),
        }
    }
    // A CA file the mode never consults is a false sense of security: say so
    // rather than encrypting without checking who answered.
    if config.root_cert.is_some() && !config.mode().verifies() {
        return Err(format!(
            "a CA file is set but sslmode={} does not verify it — use verify-ca or verify-full",
            config.mode().as_str()
        ));
    }
    let cleaned = if kept.is_empty() {
        head.to_string()
    } else {
        format!("{head}?{}", kept.join("&"))
    };
    Ok((cleaned, config))
}

/// Percent-decode a query value, leaving it alone if it is not valid encoding.
fn decode(raw: &str) -> String {
    urlencoding::decode(raw)
        .map(|decoded| decoded.into_owned())
        .unwrap_or_else(|_| raw.to_string())
}

/// A CA file must exist before a connection attempt, so the error names the
/// path instead of surfacing as a handshake failure.
fn check_root_cert(config: &SslConfig) -> Result<(), String> {
    match &config.root_cert {
        Some(path) if !path.is_file() => Err(format!(
            "CA file {} does not exist (sslrootcert / ssl-ca)",
            path.display()
        )),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod pg {
    use super::{check_root_cert, SslConfig, SslMode};
    use postgres::tls::MakeTlsConnect;
    use postgres::Socket;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::client::WebPkiServerVerifier;
    use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{
        CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore,
        SignatureScheme,
    };
    use std::sync::Arc;
    use tokio_postgres_rustls::MakeRustlsConnect;

    type Inner = MakeRustlsConnect;

    /// The connector handed to every Postgres connection.
    ///
    /// One type covers both worlds so the pool, the maintenance connection and
    /// `db.execute` all share a single `PgPool` type. The `None` arm is only
    /// reachable in `sslmode=disable`, where `tokio_postgres` returns the raw
    /// stream without ever asking the connector for anything.
    #[derive(Clone)]
    pub struct MaybeTls(Option<Inner>);

    impl std::fmt::Debug for MaybeTls {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(match self.0 {
                Some(_) => "MaybeTls(rustls)",
                None => "MaybeTls(disabled)",
            })
        }
    }

    impl MakeTlsConnect<Socket> for MaybeTls {
        type Stream = <Inner as MakeTlsConnect<Socket>>::Stream;
        type TlsConnect = <Inner as MakeTlsConnect<Socket>>::TlsConnect;
        type Error = String;

        fn make_tls_connect(&mut self, domain: &str) -> Result<Self::TlsConnect, Self::Error> {
            match self.0.as_mut() {
                // Spelled out so the delegate resolves against `Socket` rather
                // than an inferred stream type.
                Some(inner) => <Inner as MakeTlsConnect<Socket>>::make_tls_connect(inner, domain)
                    .map_err(|never| match never {}),
                None => Err("TLS is disabled for this connection (sslmode=disable)".to_string()),
            }
        }
    }

    /// Build the connector for `config`.
    pub fn connector(config: &SslConfig) -> Result<MaybeTls, String> {
        if !config.mode().encrypts() {
            return Ok(MaybeTls(None));
        }
        check_root_cert(config)?;
        Ok(MaybeTls(Some(MakeRustlsConnect::new(client_config(
            config,
        )?))))
    }

    /// The rustls client config for a mode: which verifier, and which roots.
    fn client_config(config: &SslConfig) -> Result<ClientConfig, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("postgres TLS: {e}"))?;
        let mode = config.mode();
        if !mode.verifies() {
            // libpq's `prefer` / `require`: encrypt, do not identify.
            return Ok(builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(EncryptOnly { provider }))
                .with_no_client_auth());
        }
        let roots = Arc::new(root_store(config)?);
        if mode == SslMode::VerifyFull {
            return Ok(builder.with_root_certificates(roots).with_no_client_auth());
        }
        let webpki = WebPkiServerVerifier::builder_with_provider(roots, provider)
            .build()
            .map_err(|e| format!("postgres TLS root store: {e}"))?;
        Ok(builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(ChainOnly { inner: webpki }))
            .with_no_client_auth())
    }

    /// The roots to trust: a supplied CA bundle replaces the built-in set, the
    /// way libpq's `sslrootcert` does.
    fn root_store(config: &SslConfig) -> Result<RootCertStore, String> {
        let mut roots = RootCertStore::empty();
        let Some(path) = &config.root_cert else {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            return Ok(roots);
        };
        let label = path.display();
        let mut added = 0usize;
        for cert in
            CertificateDer::pem_file_iter(path).map_err(|e| format!("CA file {label}: {e}"))?
        {
            let cert = cert.map_err(|e| format!("CA file {label}: {e}"))?;
            roots
                .add(cert)
                .map_err(|e| format!("CA file {label}: {e}"))?;
            added += 1;
        }
        if added == 0 {
            return Err(format!("CA file {label}: no certificates found"));
        }
        Ok(roots)
    }

    /// Whether a verification failure was *only* about the name on the
    /// certificate — the one check `verify-ca` deliberately skips.
    fn is_name_mismatch(err: &TlsError) -> bool {
        matches!(
            err,
            TlsError::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. }
            )
        )
    }

    /// `prefer` / `require`: the handshake still authenticates the signatures it
    /// sees, but no identity is asserted about the peer.
    #[derive(Debug)]
    struct EncryptOnly {
        provider: Arc<CryptoProvider>,
    }

    impl ServerCertVerifier for EncryptOnly {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, TlsError> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, TlsError> {
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, TlsError> {
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.provider
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    /// `verify-ca`: full chain validation, hostname mismatch forgiven. Useful
    /// for a server reached through a proxy or an IP the cert does not name.
    #[derive(Debug)]
    struct ChainOnly {
        inner: Arc<WebPkiServerVerifier>,
    }

    impl ServerCertVerifier for ChainOnly {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            intermediates: &[CertificateDer<'_>],
            server_name: &ServerName<'_>,
            ocsp_response: &[u8],
            now: UnixTime,
        ) -> Result<ServerCertVerified, TlsError> {
            match self.inner.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            ) {
                Err(e) if is_name_mismatch(&e) => Ok(ServerCertVerified::assertion()),
                other => other,
            }
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, TlsError> {
            self.inner.verify_tls12_signature(message, cert, dss)
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, TlsError> {
            self.inner.verify_tls13_signature(message, cert, dss)
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.inner.supported_verify_schemes()
        }
    }

    /// Map a mode onto what `tokio_postgres` itself negotiates. Everything from
    /// `require` up is `Require` there; the extra checks are the verifier's job.
    pub fn driver_ssl_mode(mode: SslMode) -> postgres::config::SslMode {
        match mode {
            SslMode::Disable => postgres::config::SslMode::Disable,
            SslMode::Prefer => postgres::config::SslMode::Prefer,
            SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => {
                postgres::config::SslMode::Require
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Write;

        fn cfg(mode: SslMode) -> SslConfig {
            SslConfig {
                mode: Some(mode),
                root_cert: None,
            }
        }

        #[test]
        fn disable_builds_a_connector_that_refuses_to_be_used() {
            let mut tls = connector(&cfg(SslMode::Disable)).expect("connector");
            // Unreachable in Disable mode (tokio-postgres short-circuits), but
            // if it ever were reached it must fail loudly, never silently.
            let Err(err) = tls.make_tls_connect("example.com") else {
                panic!("a disabled connector must not hand out a TLS connect");
            };
            assert!(err.contains("sslmode=disable"), "{err}");
        }

        #[test]
        fn every_encrypting_mode_builds_a_usable_connector() {
            for mode in [SslMode::Prefer, SslMode::Require, SslMode::VerifyFull] {
                let mut tls = connector(&cfg(mode)).unwrap_or_else(|e| panic!("{mode:?}: {e}"));
                assert!(
                    tls.make_tls_connect("example.com").is_ok(),
                    "{mode:?} should hand out a connector"
                );
            }
        }

        #[test]
        fn verify_ca_wraps_the_webpki_verifier_and_keeps_the_default_roots() {
            let config = client_config(&cfg(SslMode::VerifyCa)).expect("config");
            // Built from the real Mozilla root set, not an empty store.
            assert!(!webpki_roots::TLS_SERVER_ROOTS.is_empty());
            assert!(config.alpn_protocols.is_empty());
        }

        #[test]
        fn a_missing_ca_file_names_the_path() {
            let config = SslConfig {
                mode: Some(SslMode::VerifyFull),
                root_cert: Some("/no/such/ca.pem".into()),
            };
            let err = connector(&config).unwrap_err();
            assert!(err.contains("/no/such/ca.pem"), "{err}");
        }

        #[test]
        fn a_ca_file_with_no_certificates_is_rejected() {
            let dir = std::env::temp_dir().join("soli_tls_empty_ca");
            std::fs::create_dir_all(&dir).expect("dir");
            let path = dir.join("ca.pem");
            let mut file = std::fs::File::create(&path).expect("create");
            file.write_all(b"not a certificate\n").expect("write");
            let config = SslConfig {
                mode: Some(SslMode::VerifyCa),
                root_cert: Some(path.clone()),
            };
            let err = connector(&config).unwrap_err();
            assert!(err.contains("CA file"), "{err}");
            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn name_mismatch_is_the_only_error_verify_ca_forgives() {
            assert!(is_name_mismatch(&TlsError::InvalidCertificate(
                CertificateError::NotValidForName
            )));
            assert!(is_name_mismatch(&TlsError::InvalidCertificate(
                CertificateError::NotValidForNameContext {
                    expected: ServerName::try_from("db.example.com").unwrap().to_owned(),
                    presented: vec!["other.example.com".to_string()],
                }
            )));
            assert!(!is_name_mismatch(&TlsError::InvalidCertificate(
                CertificateError::Expired
            )));
            assert!(!is_name_mismatch(&TlsError::InvalidCertificate(
                CertificateError::UnknownIssuer
            )));
        }

        #[test]
        fn verify_modes_still_negotiate_require_at_the_protocol_level() {
            use postgres::config::SslMode as Driver;
            assert_eq!(driver_ssl_mode(SslMode::Disable), Driver::Disable);
            assert_eq!(driver_ssl_mode(SslMode::Prefer), Driver::Prefer);
            assert_eq!(driver_ssl_mode(SslMode::Require), Driver::Require);
            assert_eq!(driver_ssl_mode(SslMode::VerifyCa), Driver::Require);
            assert_eq!(driver_ssl_mode(SslMode::VerifyFull), Driver::Require);
        }
    }
}

#[cfg(feature = "postgres")]
pub use pg::{
    connector as postgres_connector, driver_ssl_mode as postgres_driver_ssl_mode, MaybeTls,
};

// ---------------------------------------------------------------------------
// MySQL
// ---------------------------------------------------------------------------

/// The `mysql` crate's TLS options for `config`, or `None` for cleartext.
///
/// The driver owns the rustls plumbing on this side (Mozilla roots, an optional
/// CA file, and the two "danger" switches), so the mapping is just the mode
/// ladder: encryption below `verify-ca`, chain checking at `verify-ca`, and the
/// hostname check only at `verify-full`.
#[cfg(feature = "mysql")]
pub fn mysql_ssl_opts(config: &SslConfig) -> Result<Option<mysql::SslOpts>, String> {
    let mode = config.mode();
    if !mode.encrypts() {
        return Ok(None);
    }
    check_root_cert(config)?;
    Ok(Some(
        mysql::SslOpts::default()
            .with_danger_accept_invalid_certs(!mode.verifies())
            .with_danger_skip_domain_validation(mode != SslMode::VerifyFull)
            .with_root_cert_path(config.root_cert.clone()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_vocabularies() {
        for (raw, expected) in [
            ("disable", SslMode::Disable),
            ("DISABLED", SslMode::Disable),
            ("prefer", SslMode::Prefer),
            ("PREFERRED", SslMode::Prefer),
            ("require", SslMode::Require),
            ("REQUIRED", SslMode::Require),
            ("verify-ca", SslMode::VerifyCa),
            ("VERIFY_CA", SslMode::VerifyCa),
            ("verify-full", SslMode::VerifyFull),
            ("VERIFY_IDENTITY", SslMode::VerifyFull),
        ] {
            assert_eq!(SslMode::parse(raw), Ok(expected), "{raw}");
        }
    }

    #[test]
    fn an_unknown_mode_lists_the_real_ones() {
        let err = SslMode::parse("verify_everything").unwrap_err();
        assert!(err.contains("verify-full"), "{err}");
    }

    #[test]
    fn a_url_without_a_query_string_is_untouched() {
        let (url, config) = split_url("postgres://u:p@host:5432/app").expect("split");
        assert_eq!(url, "postgres://u:p@host:5432/app");
        assert_eq!(config.mode, None);
        // No explicit mode still means opportunistic TLS.
        assert_eq!(config.mode(), SslMode::Prefer);
    }

    #[test]
    fn ssl_params_are_lifted_out_and_the_rest_survives_in_order() {
        let (url, config) = split_url(
            "mysql://root@127.0.0.1:3306/app?pool_min=1&ssl-mode=VERIFY_CA&ssl-ca=/etc/ca.pem&compress=true",
        )
        .expect("split");
        assert_eq!(
            url,
            "mysql://root@127.0.0.1:3306/app?pool_min=1&compress=true"
        );
        assert_eq!(config.mode, Some(SslMode::VerifyCa));
        assert_eq!(config.root_cert, Some(PathBuf::from("/etc/ca.pem")));
    }

    #[test]
    fn the_only_param_leaves_no_dangling_question_mark() {
        let (url, config) = split_url("postgres://u@host/app?sslmode=require").expect("split");
        assert_eq!(url, "postgres://u@host/app");
        assert_eq!(config.mode, Some(SslMode::Require));
    }

    #[test]
    fn a_percent_encoded_ca_path_is_decoded() {
        let (_, config) =
            split_url("postgres://u@host/app?sslmode=verify-full&sslrootcert=/etc/my%20ca.pem")
                .expect("split");
        assert_eq!(config.root_cert, Some(PathBuf::from("/etc/my ca.pem")));
    }

    #[test]
    fn a_ca_file_a_mode_would_ignore_is_refused() {
        let err =
            split_url("postgres://u@host/app?sslmode=require&sslrootcert=/etc/ca.pem").unwrap_err();
        assert!(err.contains("verify-ca"), "{err}");
        // ...and with no mode at all, where the default would also ignore it.
        assert!(split_url("postgres://u@host/app?ssl-ca=/etc/ca.pem").is_err());
    }

    #[test]
    fn an_invalid_mode_in_a_url_fails_the_url() {
        let err = split_url("postgres://u@host/app?sslmode=maybe").unwrap_err();
        assert!(err.contains("unknown TLS mode"), "{err}");
    }

    #[test]
    fn only_prefer_may_fall_back_to_cleartext() {
        assert!(SslMode::Prefer.may_fall_back());
        for mode in [SslMode::Require, SslMode::VerifyCa, SslMode::VerifyFull] {
            assert!(!mode.may_fall_back(), "{mode:?} must not fall back");
        }
    }

    #[test]
    fn encryption_and_verification_are_separate_rungs() {
        assert!(!SslMode::Disable.encrypts());
        assert!(SslMode::Prefer.encrypts() && !SslMode::Prefer.verifies());
        assert!(SslMode::Require.encrypts() && !SslMode::Require.verifies());
        assert!(SslMode::VerifyCa.verifies());
        assert!(SslMode::VerifyFull.verifies());
    }

    #[cfg(feature = "mysql")]
    #[test]
    fn mysql_options_follow_the_ladder() {
        let opts = |mode| {
            mysql_ssl_opts(&SslConfig {
                mode: Some(mode),
                root_cert: None,
            })
            .expect("opts")
        };
        assert!(opts(SslMode::Disable).is_none());

        let require = opts(SslMode::Require).expect("some");
        assert!(require.accept_invalid_certs());
        assert!(require.skip_domain_validation());

        let verify_ca = opts(SslMode::VerifyCa).expect("some");
        assert!(!verify_ca.accept_invalid_certs());
        assert!(verify_ca.skip_domain_validation());

        let verify_full = opts(SslMode::VerifyFull).expect("some");
        assert!(!verify_full.accept_invalid_certs());
        assert!(!verify_full.skip_domain_validation());
    }

    #[cfg(feature = "mysql")]
    #[test]
    fn mysql_carries_the_ca_path_through() {
        let config = SslConfig {
            mode: Some(SslMode::VerifyFull),
            root_cert: Some(PathBuf::from("/no/such/ca.pem")),
        };
        let err = mysql_ssl_opts(&config).unwrap_err();
        assert!(err.contains("/no/such/ca.pem"), "{err}");
    }
}
