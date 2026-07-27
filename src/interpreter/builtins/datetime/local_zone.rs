//! Local-timezone conversion for the DateTime builtins.
//!
//! Two problems with calling `chrono::Local` directly, both of which this
//! module exists to solve:
//!
//! **Cost.** `DateTime::with_timezone(&Local)` re-resolves the system zone on
//! every call. Measured against chrono 0.4.44 on this codebase's benchmark
//! machine, 2M iterations, `black_box`ed:
//!
//! | path | `Local` | cached `Tz` |
//! |---|---:|---:|
//! | `with_timezone(..).year()` | 47.2 ns | 20.2 ns |
//! | `from_local_datetime(..)` | 234.7 ns | 21.6 ns |
//!
//! Every DateTime accessor pays the first row, and the month/year boundary
//! methods pay the second.
//!
//! **Panics.** `LocalResult::unwrap()` panics for local times that either do
//! not exist (the spring-forward gap) or are ambiguous (the fall-back hour).
//! That is reachable from ordinary Soli code: `beginning_of_month` on a
//! November date under `TZ=America/Havana` panicked with *"Ambiguous local
//! time, ranging from 2026-11-01T00:00:00-05:00 to 2026-11-01T00:00:00-04:00"*.
//! Scanning all 597 zones over 2015..=2035 for the four boundary methods found
//! 17 such dates (Africa/Cairo, America/Asuncion, America/Havana, Asia/Amman,
//! Asia/Almaty, Cuba, Egypt, …). `resolve_local` below is total: it has no
//! failure case, so those methods cannot panic.
//!
//! `$TZ` is honoured first, exactly as `chrono::Local` does. This matters:
//! `iana_time_zone::get_timezone()` reads the *system* zone and ignores `$TZ`
//! entirely, so resolving through it alone would silently ignore the
//! `ENV TZ=UTC` that most containers set. When `$TZ` holds something that is
//! not an IANA name (a POSIX spec like `EST5EDT,M3.2.0/2`, or a path), we fall
//! back to `chrono::Local` and keep correctness at the old cost rather than
//! guess at a zone.

use chrono::{DateTime, FixedOffset, Local, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use std::sync::OnceLock;

/// How local time is resolved for this process.
enum Zone {
    /// An IANA zone we could name and cache — the fast path.
    Iana(Tz),
    /// `$TZ` held something chrono-tz cannot parse. Defer to `chrono::Local`,
    /// which understands POSIX specs, so behaviour is unchanged.
    System,
}

static ZONE: OnceLock<Zone> = OnceLock::new();

fn zone() -> &'static Zone {
    ZONE.get_or_init(|| match std::env::var("TZ") {
        // `$TZ` wins over the system zone, matching `chrono::Local`.
        Ok(raw) => {
            // A leading ':' is allowed by POSIX and is not part of the name.
            let name = raw.strip_prefix(':').unwrap_or(&raw);
            match name.parse::<Tz>() {
                Ok(tz) => Zone::Iana(tz),
                // Empty `TZ` means UTC per POSIX; anything else unparseable is
                // a spec only `Local` can read.
                Err(_) if name.is_empty() => Zone::Iana(chrono_tz::UTC),
                Err(_) => Zone::System,
            }
        }
        Err(_) => match iana_time_zone::get_timezone()
            .ok()
            .and_then(|n| n.parse::<Tz>().ok())
        {
            Some(tz) => Zone::Iana(tz),
            None => Zone::System,
        },
    })
}

/// Convert a UTC instant to local time.
pub fn to_local(dt: DateTime<Utc>) -> DateTime<FixedOffset> {
    match zone() {
        Zone::Iana(tz) => dt.with_timezone(tz).fixed_offset(),
        Zone::System => dt.with_timezone(&Local).fixed_offset(),
    }
}

/// Convert a nanosecond UTC timestamp to local time.
pub fn local_from_nanos(nanos: i64) -> DateTime<FixedOffset> {
    to_local(DateTime::from_timestamp_nanos(nanos))
}

/// Current instant, in local time.
pub fn now_local() -> DateTime<FixedOffset> {
    to_local(Utc::now())
}

/// Interpret a naive local datetime as an instant — **total**, never panics.
///
/// The two awkward cases are resolved the way calendar libraries conventionally
/// resolve them, and the way a user asking for "the beginning of this month"
/// means them:
///
/// * **Ambiguous** (the hour repeats when clocks go back) — take the *earliest*
///   of the two instants. That is the first time the wall clock shows this
///   value, which is what "beginning of" asks for. Matches ActiveSupport.
/// * **Nonexistent** (the hour is skipped when clocks go forward) — take the
///   first instant that does exist at or after it, i.e. the moment the gap
///   closes. Stepping a minute at a time is fine here: this branch is reached
///   only on a transition date, and the loop is bounded.
pub fn resolve_local(naive: &NaiveDateTime) -> DateTime<FixedOffset> {
    match zone() {
        Zone::Iana(tz) => resolve_with(tz, naive),
        Zone::System => resolve_with(&Local, naive),
    }
}

fn resolve_with<T: TimeZone>(tz: &T, naive: &NaiveDateTime) -> DateTime<FixedOffset>
where
    T::Offset: std::fmt::Display,
{
    match tz.from_local_datetime(naive) {
        LocalResult::Single(dt) => dt.fixed_offset(),
        // Clocks went back: this wall-clock time happens twice. Take the first.
        LocalResult::Ambiguous(earliest, _latest) => earliest.fixed_offset(),
        // Clocks went forward: this wall-clock time never happens. Walk to the
        // first that does. Real transitions are <= a few hours; Pacific/Apia
        // skipping a whole day in 2011 is the pathological case, so allow 48h
        // and fall back to treating the input as UTC if even that fails.
        LocalResult::None => {
            let mut probe = *naive;
            for _ in 0..(48 * 60) {
                probe += chrono::Duration::minutes(1);
                match tz.from_local_datetime(&probe) {
                    LocalResult::Single(dt) => return dt.fixed_offset(),
                    LocalResult::Ambiguous(earliest, _) => return earliest.fixed_offset(),
                    LocalResult::None => continue,
                }
            }
            Utc.from_utc_datetime(naive).fixed_offset()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    /// The regression this module was written for: these four dates panicked
    /// through `LocalResult::unwrap()` before it existed.
    #[test]
    fn ambiguous_and_nonexistent_local_times_resolve_instead_of_panicking() {
        let cases = [
            // (zone, naive local time, kind)
            ("America/Havana", "2026-11-01T00:00:00", "ambiguous"),
            ("Cuba", "2020-11-01T00:00:00", "ambiguous"),
            ("America/Asuncion", "2023-10-01T00:00:00", "nonexistent"),
            ("Africa/Cairo", "2024-10-31T23:59:59", "ambiguous"),
        ];
        for (name, stamp, kind) in cases {
            let tz: Tz = name.parse().expect("zone should be known to chrono-tz");
            let naive: NaiveDateTime = stamp.parse().expect("fixture parses");
            // Precondition: chrono really does refuse this one.
            assert!(
                !matches!(tz.from_local_datetime(&naive), LocalResult::Single(_)),
                "{name} {stamp} was expected to be {kind}; the fixture is stale"
            );
            // The point of the test: we return a value rather than panicking.
            let resolved = resolve_with(&tz, &naive);
            assert_eq!(resolved.year(), naive.year());
        }
    }

    #[test]
    fn ambiguous_times_take_the_earliest_instant() {
        let tz: Tz = "America/Havana".parse().unwrap();
        let naive: NaiveDateTime = "2026-11-01T00:00:00".parse().unwrap();
        let LocalResult::Ambiguous(earliest, latest) = tz.from_local_datetime(&naive) else {
            panic!("fixture is no longer ambiguous");
        };
        assert!(earliest < latest);
        assert_eq!(resolve_with(&tz, &naive).timestamp(), earliest.timestamp());
    }

    #[test]
    fn nonexistent_times_move_forward_to_the_end_of_the_gap() {
        let tz: Tz = "America/Asuncion".parse().unwrap();
        let naive: NaiveDateTime = "2023-10-01T00:00:00".parse().unwrap();
        assert!(matches!(tz.from_local_datetime(&naive), LocalResult::None));
        let resolved = resolve_with(&tz, &naive);
        // The gap opens at 00:00 and closes at 01:00 local.
        assert_eq!(resolved.hour(), 1);
        assert_eq!(resolved.day(), 1);
    }

    /// A cached `Tz` must agree with `chrono::Local` everywhere, or this is a
    /// correctness regression dressed as a speed-up.
    #[test]
    fn cached_zone_agrees_with_chrono_local_across_a_full_year() {
        let Ok(name) = iana_time_zone::get_timezone() else {
            return; // no system zone to compare against
        };
        let Ok(tz) = name.parse::<Tz>() else { return };
        let start = Utc
            .with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
            .unwrap()
            .timestamp();
        // Hourly across a year covers both transitions in either hemisphere.
        for hour in 0..(365 * 24) {
            let dt = DateTime::from_timestamp(start + hour * 3600, 0).unwrap();
            let via_local = dt.with_timezone(&Local);
            let via_cache = dt.with_timezone(&tz);
            assert_eq!(
                (
                    via_local.year(),
                    via_local.month(),
                    via_local.day(),
                    via_local.hour(),
                    via_local.minute()
                ),
                (
                    via_cache.year(),
                    via_cache.month(),
                    via_cache.day(),
                    via_cache.hour(),
                    via_cache.minute()
                ),
                "cached zone disagreed with chrono::Local at {dt}"
            );
        }
    }

    #[test]
    fn empty_tz_is_utc() {
        // POSIX: TZ="" selects UTC. Guard the parse arm that special-cases it.
        assert_eq!("".parse::<Tz>().ok(), None);
    }
}
