//! Head sampling for OpenTelemetry traces: `OTEL_TRACES_SAMPLER` and
//! `OTEL_TRACES_SAMPLER_ARG`, as the OTel SDK environment spec defines them.
//!
//! | `OTEL_TRACES_SAMPLER`        | decision                                         |
//! |------------------------------|--------------------------------------------------|
//! | `always_on`                  | every trace                                      |
//! | `always_off`                 | no trace                                         |
//! | `traceidratio`               | `ratio` of traces, decided from the trace id     |
//! | `parentbased_always_on` *(default)* | inbound sampled flag; root → sampled    |
//! | `parentbased_always_off`     | inbound sampled flag; root → not sampled         |
//! | `parentbased_traceidratio`   | inbound sampled flag; root → ratio               |
//!
//! `OTEL_TRACES_SAMPLER_ARG` is the ratio (0..=1, default 1.0) for the two ratio
//! samplers. An unknown sampler name or an unreadable ratio prints one warning
//! on stderr and falls back to the default.
//!
//! The ratio decision is deterministic from the trace id — the same rule as the
//! Go and Java SDKs (the last 8 bytes of the id, big-endian, shifted right one
//! bit, below `ratio × 2⁶³`) — so every service that samples one trace id with
//! the same ratio agrees, and a retried request decides the same way.
//!
//! A trace that is not sampled still gets ids (logs keep their `trace_id`) and
//! still propagates a `traceparent`, with flags `00`; it is only not exported.

use std::sync::OnceLock;

/// The root-span rule of a sampler, and whether a parent's decision overrides it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sampler {
    AlwaysOn,
    AlwaysOff,
    TraceIdRatio(f64),
    ParentBased(Root),
}

/// What a parent-based sampler does for a trace that has no parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Root {
    AlwaysOn,
    AlwaysOff,
    TraceIdRatio(f64),
}

impl Default for Sampler {
    fn default() -> Self {
        Sampler::ParentBased(Root::AlwaysOn)
    }
}

impl Sampler {
    /// Read a sampler from the two variables' values. `Err` carries the
    /// warning to print; the sampler to use then is the default.
    pub fn parse(name: Option<&str>, arg: Option<&str>) -> Result<Self, String> {
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        let Some(name) = name else {
            return Ok(Sampler::default());
        };
        let ratio = || -> Result<f64, String> {
            match arg.map(str::trim).filter(|a| !a.is_empty()) {
                None => Ok(1.0),
                Some(raw) => match raw.parse::<f64>() {
                    Ok(r) if (0.0..=1.0).contains(&r) => Ok(r),
                    _ => Err(format!(
                        "OTEL_TRACES_SAMPLER_ARG={raw:?} is not a ratio between 0 and 1"
                    )),
                },
            }
        };
        match name.to_ascii_lowercase().as_str() {
            "always_on" => Ok(Sampler::AlwaysOn),
            "always_off" => Ok(Sampler::AlwaysOff),
            "traceidratio" => Ok(Sampler::TraceIdRatio(ratio()?)),
            "parentbased_always_on" => Ok(Sampler::ParentBased(Root::AlwaysOn)),
            "parentbased_always_off" => Ok(Sampler::ParentBased(Root::AlwaysOff)),
            "parentbased_traceidratio" => Ok(Sampler::ParentBased(Root::TraceIdRatio(ratio()?))),
            other => Err(format!(
                "OTEL_TRACES_SAMPLER={other:?} is not one of always_on, always_off, \
                 traceidratio, parentbased_always_on, parentbased_always_off, \
                 parentbased_traceidratio"
            )),
        }
    }

    /// Whether to sample a trace. `parent_sampled` is the inbound
    /// `traceparent`'s sampled flag, `None` when the trace starts here.
    pub fn should_sample(&self, trace_id: &str, parent_sampled: Option<bool>) -> bool {
        match *self {
            Sampler::AlwaysOn => true,
            Sampler::AlwaysOff => false,
            Sampler::TraceIdRatio(ratio) => ratio_sampled(trace_id, ratio),
            Sampler::ParentBased(root) => match parent_sampled {
                Some(sampled) => sampled,
                None => match root {
                    Root::AlwaysOn => true,
                    Root::AlwaysOff => false,
                    Root::TraceIdRatio(ratio) => ratio_sampled(trace_id, ratio),
                },
            },
        }
    }
}

/// The trace-id ratio rule of the Go and Java SDKs.
fn ratio_sampled(trace_id: &str, ratio: f64) -> bool {
    if ratio >= 1.0 {
        return true;
    }
    if ratio <= 0.0 {
        return false;
    }
    let Some(low) = trace_id
        .get(16..32)
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
    else {
        return false;
    };
    let bound = (ratio * (1u64 << 63) as f64) as u64;
    (low >> 1) < bound
}

/// The process's sampler, read from the environment once.
pub fn sampler() -> &'static Sampler {
    static SAMPLER: OnceLock<Sampler> = OnceLock::new();
    SAMPLER.get_or_init(|| {
        let name = crate::platform::env::var("OTEL_TRACES_SAMPLER").ok();
        let arg = crate::platform::env::var("OTEL_TRACES_SAMPLER_ARG").ok();
        Sampler::parse(name.as_deref(), arg.as_deref()).unwrap_or_else(|warning| {
            eprintln!("[WARN] {warning}; using parentbased_always_on");
            Sampler::default()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOW: &str = "0af7651916cd43dd0000000000000001";
    const HIGH: &str = "0af7651916cd43ddffffffffffffffff";

    #[test]
    fn the_default_is_parent_based_always_on() {
        assert_eq!(
            Sampler::parse(None, None),
            Ok(Sampler::ParentBased(Root::AlwaysOn))
        );
        assert_eq!(
            Sampler::parse(Some("  "), None),
            Ok(Sampler::ParentBased(Root::AlwaysOn))
        );
    }

    #[test]
    fn every_spec_name_parses() {
        assert_eq!(
            Sampler::parse(Some("always_on"), None),
            Ok(Sampler::AlwaysOn)
        );
        assert_eq!(
            Sampler::parse(Some("ALWAYS_OFF"), None),
            Ok(Sampler::AlwaysOff)
        );
        assert_eq!(
            Sampler::parse(Some("traceidratio"), Some("0.25")),
            Ok(Sampler::TraceIdRatio(0.25))
        );
        assert_eq!(
            Sampler::parse(Some("traceidratio"), None),
            Ok(Sampler::TraceIdRatio(1.0))
        );
        assert_eq!(
            Sampler::parse(Some("parentbased_always_off"), None),
            Ok(Sampler::ParentBased(Root::AlwaysOff))
        );
        assert_eq!(
            Sampler::parse(Some("parentbased_traceidratio"), Some("0.1")),
            Ok(Sampler::ParentBased(Root::TraceIdRatio(0.1)))
        );
    }

    #[test]
    fn invalid_values_are_refused_with_a_reason() {
        assert!(Sampler::parse(Some("sometimes"), None)
            .unwrap_err()
            .contains("sometimes"));
        assert!(Sampler::parse(Some("traceidratio"), Some("1.5"))
            .unwrap_err()
            .contains("1.5"));
        assert!(Sampler::parse(Some("parentbased_traceidratio"), Some("half")).is_err());
        assert!(Sampler::parse(Some("traceidratio"), Some("-0.1")).is_err());
    }

    #[test]
    fn always_on_and_off_ignore_the_parent() {
        assert!(Sampler::AlwaysOn.should_sample(LOW, Some(false)));
        assert!(!Sampler::AlwaysOff.should_sample(LOW, Some(true)));
    }

    #[test]
    fn parent_based_follows_the_inbound_flag() {
        for root in [Root::AlwaysOn, Root::AlwaysOff, Root::TraceIdRatio(0.5)] {
            let sampler = Sampler::ParentBased(root);
            assert!(sampler.should_sample(HIGH, Some(true)), "{root:?}");
            assert!(!sampler.should_sample(LOW, Some(false)), "{root:?}");
        }
        assert!(Sampler::ParentBased(Root::AlwaysOn).should_sample(HIGH, None));
        assert!(!Sampler::ParentBased(Root::AlwaysOff).should_sample(LOW, None));
    }

    #[test]
    fn the_ratio_decision_comes_from_the_trace_id() {
        let half = Sampler::TraceIdRatio(0.5);
        assert!(half.should_sample(LOW, None));
        assert!(!half.should_sample(HIGH, None));
        // Deterministic: the same id always gets the same answer.
        for _ in 0..10 {
            assert!(half.should_sample(LOW, None));
            assert!(!half.should_sample(HIGH, None));
        }
        assert!(Sampler::TraceIdRatio(1.0).should_sample(HIGH, None));
        assert!(!Sampler::TraceIdRatio(0.0).should_sample(LOW, None));
        let root_ratio = Sampler::ParentBased(Root::TraceIdRatio(0.5));
        assert!(root_ratio.should_sample(LOW, None));
        assert!(!root_ratio.should_sample(HIGH, None));
    }

    #[test]
    fn the_ratio_holds_over_many_ids() {
        let sampler = Sampler::TraceIdRatio(0.25);
        let sampled = (0..4000u64)
            .filter(|i| {
                let low = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                sampler.should_sample(&format!("0af7651916cd43dd{low:016x}"), None)
            })
            .count();
        assert!((800..1200).contains(&sampled), "{sampled} of 4000");
    }
}
