//! Where metrics go, read from the environment once.
//!
//! Every parser here takes a lookup function rather than reading the process
//! environment, so the precedence rules are tested without touching it.
//!
//! The service identity, the OTLP resource and the OTLP headers are not
//! resolved here: they come from [`crate::serve::otel::OtelConfig`], the one
//! resolution traces and logs use too — `OTEL_SERVICE_NAME` >
//! `service.name` in `OTEL_RESOURCE_ATTRIBUTES` > `DD_SERVICE` > `soli`,
//! explicit resource attributes over `DD_ENV` / `DD_VERSION`, and
//! `OTEL_EXPORTER_OTLP_HEADERS` overridden by
//! `OTEL_EXPORTER_OTLP_METRICS_HEADERS` (percent-decoded, CR / LF / NUL
//! rejected, warnings that never print a value). So the three signals and
//! the DogStatsD tags agree on who is reporting.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::registry::{parse_buckets, DEFAULT_BUCKETS};
use crate::serve::otel::{OtelConfig, ServiceIdentity, Signal};

/// A lookup function: the environment, or a table in a test.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

const DEFAULT_STATSD_PORT: u16 = 8125;
const DEFAULT_OTLP_METRICS: &str = "http://127.0.0.1:4318/v1/metrics";
const DEFAULT_EXPORT_INTERVAL_MS: u64 = 60_000;
const DEFAULT_EXPORT_TIMEOUT_MS: u64 = 10_000;

#[derive(Clone, Debug, PartialEq)]
pub enum StatsdTarget {
    /// `host:port`, resolved when the sender starts.
    Udp(String),
    /// A Unix datagram socket path.
    Unix(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatsdConfig {
    pub target: StatsdTarget,
    /// `env:prod`, `service:shop`, `version:1.2`, then `DD_TAGS`.
    pub global_tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OtlpConfig {
    pub endpoint: String,
    /// Never logged: they usually carry an API key.
    pub headers: Vec<(String, String)>,
    pub interval: Duration,
    pub timeout: Duration,
    pub resource: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct MetricsConfig {
    /// `SOLI_METRICS=1`: Prometheus text at `/_metrics` is worth filling.
    pub prometheus: bool,
    pub statsd: Option<StatsdConfig>,
    pub otlp: Option<OtlpConfig>,
    /// Histogram buckets, seconds (`SOLI_METRICS_BUCKETS`).
    pub buckets: Arc<[f64]>,
}

impl MetricsConfig {
    /// Whether anything reads the metrics: the built-in series are timed and
    /// app metrics recorded only then.
    pub fn any_sink(&self) -> bool {
        self.prometheus || self.statsd.is_some() || self.otlp.is_some()
    }

    /// Resolve from `get` alone — the OTel side too (its warnings dropped).
    /// Tests use this; the process uses [`config`], which shares the
    /// already-resolved [`crate::serve::otel::config`].
    pub fn from_lookup(get: Lookup) -> Self {
        let (otel, _warnings) = OtelConfig::from_lookup(get);
        Self::resolve(get, &otel)
    }

    fn resolve(get: Lookup, otel: &OtelConfig) -> Self {
        let buckets = match get("SOLI_METRICS_BUCKETS") {
            Some(raw) if !raw.trim().is_empty() => parse_buckets(&raw).unwrap_or_else(|| {
                eprintln!(
                    "[WARN] SOLI_METRICS_BUCKETS must be increasing finite numbers separated by commas; using the defaults"
                );
                DEFAULT_BUCKETS.into()
            }),
            _ => DEFAULT_BUCKETS.into(),
        };
        Self {
            prometheus: truthy(get("SOLI_METRICS")),
            statsd: statsd_from(get, &otel.identity),
            otlp: otlp_from(get, otel),
            buckets,
        }
    }
}

/// The process' metrics configuration.
pub fn config() -> &'static MetricsConfig {
    static CFG: OnceLock<MetricsConfig> = OnceLock::new();
    CFG.get_or_init(|| {
        MetricsConfig::resolve(
            &|name| crate::platform::env::var(name).ok(),
            crate::serve::otel::config(),
        )
    })
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn truthy(v: Option<String>) -> bool {
    v.is_some_and(|v| {
        let v = v.trim();
        v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
    })
}

fn falsy(v: &str) -> bool {
    let v = v.trim();
    v == "0"
        || v.eq_ignore_ascii_case("false")
        || v.eq_ignore_ascii_case("off")
        || v.eq_ignore_ascii_case("no")
}

// ---------- DogStatsD ----------

/// `SOLI_STATSD` (`host:port`, a `udp://` / `unix://` URL, or `off`), then
/// `DD_DOGSTATSD_URL`, then `DD_AGENT_HOST` + `DD_DOGSTATSD_PORT`.
pub fn statsd_from(get: Lookup, identity: &ServiceIdentity) -> Option<StatsdConfig> {
    let target = if let Some(raw) = non_empty(get("SOLI_STATSD")) {
        if falsy(&raw) {
            return None;
        }
        parse_statsd_target(&raw)
    } else if let Some(url) = non_empty(get("DD_DOGSTATSD_URL")) {
        parse_statsd_target(&url)
    } else if let Some(host) = non_empty(get("DD_AGENT_HOST")) {
        let port = non_empty(get("DD_DOGSTATSD_PORT"))
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_STATSD_PORT);
        Some(StatsdTarget::Udp(join_host_port(&host, port)))
    } else {
        None
    }?;
    Some(StatsdConfig {
        target,
        global_tags: global_tags(get, identity),
    })
}

fn join_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn parse_statsd_target(raw: &str) -> Option<StatsdTarget> {
    if let Some(path) = raw.strip_prefix("unix://") {
        if path.is_empty() {
            eprintln!("[WARN] DogStatsD: empty unix:// socket path; metrics are not sent");
            return None;
        }
        return Some(StatsdTarget::Unix(path.to_string()));
    }
    let hostport = raw
        .strip_prefix("udp://")
        .unwrap_or(raw)
        .trim_end_matches('/');
    if hostport.is_empty() || hostport.contains("://") {
        eprintln!("[WARN] DogStatsD: cannot read target \"{raw}\" (want host:port, udp://host:port or unix:///path); metrics are not sent");
        return None;
    }
    // A port is the text after the last ':' outside brackets.
    let has_port = match hostport.rfind(']') {
        Some(close) => hostport[close..].contains(':'),
        None => hostport.matches(':').count() == 1,
    };
    if has_port {
        Some(StatsdTarget::Udp(hostport.to_string()))
    } else {
        Some(StatsdTarget::Udp(join_host_port(
            hostport,
            DEFAULT_STATSD_PORT,
        )))
    }
}

/// A tag made safe for a DogStatsD line: `|`, `,` and line breaks would end
/// the tag, the tag list or the line. (A `#` inside a tag is harmless — only
/// `|#` opens the tag list — so `route:posts#show` stays as it is.)
pub fn sanitize_tag(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            '|' | ',' | '\n' | '\r' => '_',
            other => other,
        })
        .collect()
}

/// `env:`, `service:`, `version:` from the service identity the OTel side
/// resolved (`DD_ENV` / `DD_SERVICE` / `DD_VERSION`, unless
/// `OTEL_SERVICE_NAME` or `OTEL_RESOURCE_ATTRIBUTES` say otherwise; a tag
/// only when configured), then each `DD_TAGS` entry (comma- or
/// space-separated).
pub fn global_tags(get: Lookup, identity: &ServiceIdentity) -> Vec<String> {
    let mut tags = Vec::new();
    for (value, key) in [
        (&identity.env, "env"),
        (&identity.service, "service"),
        (&identity.version, "version"),
    ] {
        if let Some(v) = value {
            tags.push(sanitize_tag(&format!("{key}:{v}")));
        }
    }
    if let Some(raw) = non_empty(get("DD_TAGS")) {
        for tag in raw
            .split([',', ' '])
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            let tag = sanitize_tag(tag);
            let key = tag.split(':').next().unwrap_or("");
            // DD_ENV & co. win over the same key in DD_TAGS, as in the tracers.
            if ["env", "service", "version"].contains(&key)
                && tags.iter().any(|t| t.split(':').next() == Some(key))
            {
                continue;
            }
            tags.push(tag);
        }
    }
    tags
}

// ---------- OTLP ----------

/// `SOLI_OTEL_METRICS=1` or `OTEL_METRICS_EXPORTER=otlp`; off under
/// `OTEL_SDK_DISABLED=true` and with `SOLI_OTEL_METRICS=0`.
///
/// The resource and headers come from `otel`; the per-POST timeout is
/// `OTEL_METRIC_EXPORT_TIMEOUT`, else `OTEL_EXPORTER_OTLP_TIMEOUT`, else 10 s.
pub fn otlp_from(get: Lookup, otel: &OtelConfig) -> Option<OtlpConfig> {
    if truthy(get("OTEL_SDK_DISABLED")) {
        return None;
    }
    let soli = non_empty(get("SOLI_OTEL_METRICS"));
    let enabled = match soli.as_deref() {
        Some(v) if falsy(v) => false,
        Some(v) => truthy(Some(v.to_string())),
        None => non_empty(get("OTEL_METRICS_EXPORTER"))
            .is_some_and(|v| v.split(',').any(|e| e.trim().eq_ignore_ascii_case("otlp"))),
    };
    if !enabled {
        return None;
    }
    let endpoint = if let Some(full) = non_empty(get("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT")) {
        full
    } else if let Some(base) = non_empty(get("OTEL_EXPORTER_OTLP_ENDPOINT")) {
        let base = base.trim_end_matches('/');
        if base.ends_with("/v1/metrics") {
            base.to_string()
        } else {
            format!("{base}/v1/metrics")
        }
    } else {
        DEFAULT_OTLP_METRICS.to_string()
    };
    let millis = |name: &str, default: u64| {
        non_empty(get(name))
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .unwrap_or(default)
    };
    Some(OtlpConfig {
        endpoint,
        interval: Duration::from_millis(
            millis("OTEL_METRIC_EXPORT_INTERVAL", DEFAULT_EXPORT_INTERVAL_MS).max(100),
        ),
        timeout: if non_empty(get("OTEL_METRIC_EXPORT_TIMEOUT")).is_none()
            && non_empty(get("OTEL_EXPORTER_OTLP_TIMEOUT")).is_some()
        {
            otel.export_timeout
        } else {
            Duration::from_millis(millis(
                "OTEL_METRIC_EXPORT_TIMEOUT",
                DEFAULT_EXPORT_TIMEOUT_MS,
            ))
        },
        headers: otel.otlp_headers(Signal::Metrics).to_vec(),
        resource: otel.resource_pairs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    /// What `MetricsConfig::from_lookup` does: the OTel side resolved from
    /// the same table.
    fn otel(get: Lookup) -> OtelConfig {
        OtelConfig::from_lookup(get).0
    }

    fn statsd(get: Lookup) -> Option<StatsdConfig> {
        statsd_from(get, &otel(get).identity)
    }

    fn otlp(get: Lookup) -> Option<OtlpConfig> {
        otlp_from(get, &otel(get))
    }

    #[test]
    fn nothing_set_means_no_sink() {
        let cfg = MetricsConfig::from_lookup(&env(&[]));
        assert!(!cfg.any_sink());
        assert_eq!(&*cfg.buckets, DEFAULT_BUCKETS);
    }

    #[test]
    fn statsd_precedence() {
        let all = [
            ("SOLI_STATSD", "10.0.0.1:9125"),
            ("DD_DOGSTATSD_URL", "udp://10.0.0.2:8125"),
            ("DD_AGENT_HOST", "10.0.0.3"),
        ];
        let target = |pairs: &[(&str, &str)]| statsd(&env(pairs)).map(|c| c.target);
        assert_eq!(
            target(&all),
            Some(StatsdTarget::Udp("10.0.0.1:9125".into()))
        );
        assert_eq!(
            target(&all[1..]),
            Some(StatsdTarget::Udp("10.0.0.2:8125".into()))
        );
        assert_eq!(
            target(&all[2..]),
            Some(StatsdTarget::Udp("10.0.0.3:8125".into()))
        );
        assert_eq!(
            target(&[("DD_AGENT_HOST", "agent"), ("DD_DOGSTATSD_PORT", "18125")]),
            Some(StatsdTarget::Udp("agent:18125".into()))
        );
        assert_eq!(
            target(&[("DD_AGENT_HOST", "::1")]),
            Some(StatsdTarget::Udp("[::1]:8125".into()))
        );
        assert_eq!(
            target(&[("DD_DOGSTATSD_URL", "unix:///var/run/datadog/dsd.socket")]),
            Some(StatsdTarget::Unix("/var/run/datadog/dsd.socket".into()))
        );
        assert_eq!(
            target(&[("SOLI_STATSD", "localhost")]),
            Some(StatsdTarget::Udp("localhost:8125".into()))
        );
        // SOLI_STATSD=off silences an agent host set for the tracer.
        assert_eq!(
            target(&[("SOLI_STATSD", "off"), ("DD_AGENT_HOST", "a")]),
            None
        );
        assert_eq!(target(&[]), None);
    }

    #[test]
    fn global_tags_from_dd_env() {
        let get = env(&[
            ("DD_ENV", "prod"),
            ("DD_SERVICE", "shop"),
            ("DD_VERSION", "1.2"),
            ("DD_TAGS", "team:web, region:eu env:ignored bare"),
        ]);
        let tags = global_tags(&get, &otel(&get).identity);
        assert_eq!(
            tags,
            vec![
                "env:prod",
                "service:shop",
                "version:1.2",
                "team:web",
                "region:eu",
                "bare"
            ]
        );
        assert_eq!(sanitize_tag("a|b,c#d\ne"), "a_b_c#d_e");
    }

    #[test]
    fn global_tags_follow_the_otel_identity() {
        let get = env(&[
            ("DD_SERVICE", "dd-shop"),
            ("DD_VERSION", "1"),
            ("OTEL_SERVICE_NAME", "otel-shop"),
            ("OTEL_RESOURCE_ATTRIBUTES", "service.version=2"),
            ("DD_AGENT_HOST", "agent"),
        ]);
        let cfg = statsd(&get).unwrap();
        assert_eq!(cfg.global_tags, vec!["service:otel-shop", "version:2"]);
        // Nothing configured: no service tag (the agent derives one).
        assert!(global_tags(&env(&[]), &otel(&env(&[])).identity).is_empty());
    }

    #[test]
    fn otlp_switches_and_endpoint_precedence() {
        assert!(otlp(&env(&[])).is_none());
        assert!(otlp(&env(&[("OTEL_METRICS_EXPORTER", "none")])).is_none());
        assert!(otlp(&env(&[
            ("SOLI_OTEL_METRICS", "0"),
            ("OTEL_METRICS_EXPORTER", "otlp")
        ]))
        .is_none());
        assert!(otlp(&env(&[
            ("SOLI_OTEL_METRICS", "1"),
            ("OTEL_SDK_DISABLED", "true")
        ]))
        .is_none());

        let cfg = otlp(&env(&[("OTEL_METRICS_EXPORTER", "console,otlp")])).unwrap();
        assert_eq!(cfg.endpoint, DEFAULT_OTLP_METRICS);
        assert_eq!(cfg.interval, Duration::from_secs(60));

        let cfg = otlp(&env(&[
            ("SOLI_OTEL_METRICS", "1"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4318/"),
            ("OTEL_METRIC_EXPORT_INTERVAL", "5000"),
        ]))
        .unwrap();
        assert_eq!(cfg.endpoint, "http://collector:4318/v1/metrics");
        assert_eq!(cfg.interval, Duration::from_secs(5));

        let cfg = otlp(&env(&[
            ("SOLI_OTEL_METRICS", "true"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4318"),
            (
                "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
                "https://m.example/otlp",
            ),
        ]))
        .unwrap();
        assert_eq!(cfg.endpoint, "https://m.example/otlp");
    }

    #[test]
    fn otlp_headers_decode_and_metrics_headers_override() {
        let cfg = otlp(&env(&[
            ("SOLI_OTEL_METRICS", "1"),
            (
                "OTEL_EXPORTER_OTLP_HEADERS",
                "DD-API-KEY=abc%20def,x-team = web,broken",
            ),
            ("OTEL_EXPORTER_OTLP_METRICS_HEADERS", "dd-api-key=override"),
        ]))
        .unwrap();
        assert_eq!(
            cfg.headers,
            vec![
                ("x-team".to_string(), "web".to_string()),
                ("dd-api-key".to_string(), "override".to_string())
            ]
        );
        let cfg = otlp(&env(&[
            ("SOLI_OTEL_METRICS", "1"),
            (
                "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
                "a=b%3Dc,=x,k=line%0Abreak",
            ),
        ]))
        .unwrap();
        assert_eq!(cfg.headers, vec![("a".to_string(), "b=c".to_string())]);
    }

    #[test]
    fn export_timeout_falls_back_to_the_exporter_timeout() {
        let on = ("SOLI_OTEL_METRICS", "1");
        assert_eq!(otlp(&env(&[on])).unwrap().timeout, Duration::from_secs(10));
        let cfg = otlp(&env(&[on, ("OTEL_EXPORTER_OTLP_TIMEOUT", "3000")])).unwrap();
        assert_eq!(cfg.timeout, Duration::from_secs(3));
        let cfg = otlp(&env(&[
            on,
            ("OTEL_EXPORTER_OTLP_TIMEOUT", "3000"),
            ("OTEL_METRIC_EXPORT_TIMEOUT", "500"),
        ]))
        .unwrap();
        assert_eq!(cfg.timeout, Duration::from_millis(500));
    }

    #[test]
    fn resource_prefers_otel_service_name_then_dd() {
        let get = env(&[
            ("SOLI_OTEL_METRICS", "1"),
            ("DD_SERVICE", "dd-shop"),
            ("DD_ENV", "staging"),
            ("DD_VERSION", "3"),
            ("OTEL_RESOURCE_ATTRIBUTES", "team=web,service.version=4"),
        ]);
        let attrs = otlp(&get).unwrap().resource;
        // Exactly the resource traces and logs carry.
        assert_eq!(attrs, otel(&get).resource_pairs());
        let get = |k: &str| {
            attrs
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("service.name"), Some("dd-shop"));
        assert_eq!(get("deployment.environment.name"), Some("staging"));
        assert_eq!(get("deployment.environment"), Some("staging"));
        assert_eq!(
            get("service.version"),
            Some("4"),
            "OTEL_RESOURCE_ATTRIBUTES wins"
        );
        assert_eq!(get("team"), Some("web"));
        let attrs = otlp(&env(&[
            ("SOLI_OTEL_METRICS", "1"),
            ("DD_SERVICE", "dd"),
            ("OTEL_SERVICE_NAME", "otel"),
        ]))
        .unwrap()
        .resource;
        assert_eq!(attrs[0], ("service.name".to_string(), "otel".to_string()));
    }

    #[test]
    fn bad_buckets_fall_back_to_defaults() {
        let cfg = MetricsConfig::from_lookup(&env(&[("SOLI_METRICS_BUCKETS", "1,0")]));
        assert_eq!(&*cfg.buckets, DEFAULT_BUCKETS);
        let cfg = MetricsConfig::from_lookup(&env(&[("SOLI_METRICS_BUCKETS", "0.1,1")]));
        assert_eq!(&*cfg.buckets, &[0.1, 1.0][..]);
    }
}
