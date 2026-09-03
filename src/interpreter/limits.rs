//! Ceilings on the sizes runtime operations may allocate.
//!
//! A handler is one thread out of a small pool, and it runs with request data
//! in hand. `range(0, params["n"])` and `"x" * params["n"]` both allocate
//! eagerly and neither checked its argument, so a single request could ask for
//! gigabytes: `range(0, 9223372036854775807)` reported
//! `memory allocation of 3221225472 bytes failed` and dumped core, and
//! `"abcdefgh" * 1000000000` asked for 8 GB. An allocation failure is an
//! **abort**, not a catchable panic — the whole process goes, taking every
//! worker and every tenant with it, so this is a denial of service rather than
//! a failed request. A negative count was worse in a quieter way: `-1 as usize`
//! became `usize::MAX` and panicked in `capacity overflow`.
//!
//! The caps below are deliberately generous — far past any legitimate use — and
//! configurable, because the point is to turn an abort into an ordinary
//! catchable error, not to police application design.

/// Maximum number of elements a single `range(...)` / `a..b` may materialise.
///
/// 16 million `Value::Int`s is roughly 384 MB — already unreasonable, and still
/// an error rather than a dead process. `SOLI_MAX_RANGE_LEN` overrides it.
pub fn max_range_len() -> u64 {
    env_u64("SOLI_MAX_RANGE_LEN", 16_777_216)
}

/// Maximum length in bytes of a string built by `"x" * n`.
/// `SOLI_MAX_STRING_ALLOC_BYTES` overrides it.
pub fn max_string_alloc_bytes() -> u64 {
    env_u64("SOLI_MAX_STRING_ALLOC_BYTES", 64 * 1024 * 1024)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

/// Largest page a `paginate({"per": n})` call may request.
///
/// `per` came straight from request params with no ceiling, so `?per=100000000`
/// on any paginated index pulled and hydrated the whole collection — one
/// request, one worker, the entire table in memory. 1000 rows is already more
/// than any page a person reads. `SOLI_MAX_PAGE_SIZE` overrides it.
pub fn max_page_size() -> usize {
    std::env::var("SOLI_MAX_PAGE_SIZE")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(1000)
}

/// Clamp a requested page size to [`max_page_size`].
pub fn clamp_page_size(requested: usize) -> usize {
    requested.min(max_page_size())
}

/// Validate a `start..end` range before it is materialised.
///
/// An empty range (`end <= start`) is fine and yields nothing — that is how
/// `for i in 0..items.length()` behaves on an empty list.
pub fn check_range(start: i64, end: i64, what: &str) -> Result<(), String> {
    if end <= start {
        return Ok(());
    }
    let len = (end as i128) - (start as i128);
    let max = max_range_len() as i128;
    if len > max {
        return Err(format!(
            "{what} would build {len} elements, over the {max} limit. \
             Iterate lazily or raise SOLI_MAX_RANGE_LEN if you really need it."
        ));
    }
    Ok(())
}

/// Validate a `string * count` repetition before it is materialised.
pub fn check_string_repeat(len: usize, count: i64, what: &str) -> Result<(), String> {
    if count < 0 {
        return Err(format!(
            "{what} needs a non-negative count, got {count}. \
             A negative count used to wrap to a huge allocation."
        ));
    }
    let total = (len as u128) * (count as u128);
    let max = max_string_alloc_bytes() as u128;
    if total > max {
        return Err(format!(
            "{what} would build a {total}-byte string, over the {max} limit. \
             Raise SOLI_MAX_STRING_ALLOC_BYTES if you really need it."
        ));
    }
    Ok(())
}

/// Validate a stepped `range(start, end, step)` before it is materialised.
///
/// `step` must already be non-zero (the callers reject zero with their own
/// message).
pub fn check_range_with_step(start: i64, end: i64, step: i64, what: &str) -> Result<(), String> {
    let span = if step > 0 {
        if end <= start {
            return Ok(());
        }
        (end as i128) - (start as i128)
    } else {
        if end >= start {
            return Ok(());
        }
        (start as i128) - (end as i128)
    };
    let stride = (step as i128).unsigned_abs() as i128;
    // Ceiling division: the last element is included.
    let len = (span + stride - 1) / stride;
    let max = max_range_len() as i128;
    if len > max {
        return Err(format!(
            "{what} would build {len} elements, over the {max} limit. \
             Iterate lazily or raise SOLI_MAX_RANGE_LEN if you really need it."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_range_is_allowed() {
        assert!(check_range(0, 1000, "range()").is_ok());
        assert!(check_range(-5, 5, "range()").is_ok());
    }

    /// An empty or reversed range yields nothing and must not be an error —
    /// `for i in 0..list.length()` on an empty list is normal code.
    #[test]
    fn an_empty_range_is_not_an_error() {
        assert!(check_range(0, 0, "range()").is_ok());
        assert!(check_range(10, 3, "range()").is_ok());
    }

    /// The reported abort: `range(0, i64::MAX)` asked the allocator for
    /// gigabytes and killed the process.
    #[test]
    fn an_enormous_range_is_refused_before_allocating() {
        let err = check_range(0, i64::MAX, "range()").unwrap_err();
        assert!(err.contains("over the"), "{err}");
        assert!(err.contains("SOLI_MAX_RANGE_LEN"), "{err}");
    }

    /// The span must be computed in wider arithmetic: `end - start` on
    /// `i64::MIN..i64::MAX` overflows i64.
    #[test]
    fn a_full_width_range_does_not_overflow_the_check_itself() {
        let err = check_range(i64::MIN, i64::MAX, "range()").unwrap_err();
        assert!(err.contains("over the"), "{err}");
    }

    #[test]
    fn a_stepped_range_counts_its_real_length() {
        // 0..1_000_000 by 1000 is a thousand elements, not a million.
        assert!(check_range_with_step(0, 1_000_000, 1000, "range()").is_ok());
        // Counting down is the same size.
        assert!(check_range_with_step(1_000_000, 0, -1000, "range()").is_ok());
        // A step of 1 over a huge span is still refused.
        assert!(check_range_with_step(0, i64::MAX, 1, "range()").is_err());
        // As is a huge span with a small step.
        assert!(check_range_with_step(0, i64::MAX, 2, "range()").is_err());
    }

    #[test]
    fn a_stepped_range_going_the_wrong_way_is_empty() {
        assert!(check_range_with_step(0, i64::MAX, -1, "range()").is_ok());
        assert!(check_range_with_step(i64::MAX, 0, 1, "range()").is_ok());
    }

    #[test]
    fn ordinary_repetition_is_allowed() {
        assert!(check_string_repeat(8, 100, "string * count").is_ok());
        assert!(check_string_repeat(0, i64::MAX, "string * count").is_ok());
    }

    #[test]
    fn a_negative_count_is_refused_rather_than_wrapping() {
        let err = check_string_repeat(8, -1, "string * count").unwrap_err();
        assert!(err.contains("non-negative"), "{err}");
    }

    #[test]
    fn an_enormous_repetition_is_refused_before_allocating() {
        let err = check_string_repeat(8, 1_000_000_000, "string * count").unwrap_err();
        assert!(err.contains("over the"), "{err}");
        assert!(err.contains("SOLI_MAX_STRING_ALLOC_BYTES"), "{err}");
    }
}

#[cfg(test)]
mod page_size_tests {
    use super::*;

    /// `?per=100000000` on a paginated index used to pull and hydrate the whole
    /// collection for one request.
    #[test]
    fn an_enormous_page_is_clamped() {
        assert_eq!(clamp_page_size(100_000_000), max_page_size());
    }

    /// Ordinary page sizes are untouched, so nothing in an app changes.
    #[test]
    fn ordinary_pages_pass_through() {
        assert_eq!(clamp_page_size(25), 25);
        assert_eq!(clamp_page_size(100), 100);
        assert_eq!(clamp_page_size(max_page_size()), max_page_size());
    }
}
