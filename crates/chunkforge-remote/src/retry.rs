//! Bounded HTTP retry policy shared by [`crate::HttpChunkSource`] and
//! [`crate::HttpChunkSink`].
//!
//! Default [`RetryPolicy::max_retries`] is **0** (single attempt ≡ 0.7.0).
//! Transient statuses / transport failures are retried with exponential backoff
//! and full jitter; permanent 4xx, missing (404/410), and corrupt/hash failures
//! are never retried.

use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Bounded retries for transient HTTP failures (library default: 0 extra tries).
///
/// `max_retries` is the number of **extra** attempts after the first try
/// (`0` → exactly one attempt, byte-compatible with 0.7.0 behaviour).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Extra attempts after the first try. Default `0`.
    pub max_retries: u32,
    /// Base of the exponential backoff (default 100ms).
    pub base_backoff: Duration,
    /// Cap on backoff delay before jitter (default 2s).
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 0,
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(2),
        }
    }
}

impl RetryPolicy {
    /// Policy with the given extra-attempt budget and default backoff (100ms / 2s).
    pub fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            ..Self::default()
        }
    }

    /// Backoff delay after `failed_attempt` failures (0 = after the first failure),
    /// using exponential growth capped by [`Self::max_backoff`], then **full jitter**
    /// (uniform in `[0, cap]`).
    pub fn backoff_delay(&self, failed_attempt: u32) -> Duration {
        let shift = failed_attempt.min(16);
        let exp = self.base_backoff.saturating_mul(1u32 << shift);
        let cap = min_duration(exp, self.max_backoff);
        full_jitter(cap)
    }

    /// Sleep for [`Self::backoff_delay`] when the delay is non-zero.
    pub fn sleep_before_retry(&self, failed_attempt: u32) {
        let d = self.backoff_delay(failed_attempt);
        if !d.is_zero() {
            std::thread::sleep(d);
        }
    }
}

fn min_duration(a: Duration, b: Duration) -> Duration {
    if a < b { a } else { b }
}

/// Full jitter: uniform random delay in `[0, cap]`.
///
/// Uses a cheap std-only mix of wall-clock nanos (no `rand` dependency). When
/// `cap` is zero the delay is zero (useful for fast unit tests).
fn full_jitter(cap: Duration) -> Duration {
    if cap.is_zero() {
        return Duration::ZERO;
    }
    let nanos = cap.as_nanos();
    if nanos == 0 {
        return Duration::ZERO;
    }
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // xorshift64-ish mix
    let mut x = (seed ^ (seed >> 32)) as u64;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    let r = (x as u128) % (nanos + 1);
    Duration::from_nanos(r as u64)
}

/// Whether an HTTP status code is a **transient** failure eligible for retry.
///
/// Transient: **408** Request Timeout, **429** Too Many Requests, **500–504**
/// server errors, and CDN-style **520–524**.
///
/// **Not** transient (do not retry): 404/410 Missing, 401/403 and other
/// permanent 4xx, success (2xx), redirects (3xx).
pub fn http_status_is_transient(code: u16) -> bool {
    matches!(code, 408 | 429 | 500..=504 | 520..=524)
}

/// Classify a ureq transport / status error as transient for retry purposes.
///
/// Transient: [`http_status_is_transient`] statuses, timeouts, connection
/// failures, and connection-reset-like I/O errors.
///
/// Permanent (among others): 404/410, 401/403 and other non-transient statuses,
/// bad URI / protocol / TLS config errors.
pub fn ureq_error_is_transient(err: &ureq::Error) -> bool {
    match err {
        ureq::Error::StatusCode(code) => http_status_is_transient(*code),
        ureq::Error::Timeout(_) => true,
        ureq::Error::ConnectionFailed => true,
        ureq::Error::Io(e) => io_error_is_transient(e),
        // DNS miss / bad request construction / TLS config — not worth retrying.
        _ => false,
    }
}

fn io_error_is_transient(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::TimedOut
            | io::ErrorKind::Interrupted
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::NotConnected
            | io::ErrorKind::WouldBlock
            // ConnectionRefused can be a restarting peer.
            | io::ErrorKind::ConnectionRefused
    )
}

/// Outcome of one attempt inside [`run_with_retry`].
pub(crate) enum Attempt<T, E> {
    Ok(T),
    /// Do not retry (missing / permanent / corrupt / exhausted policy elsewhere).
    Fatal(E),
    /// Eligible for another try if the policy still has budget.
    Transient(E),
}

/// Run `f` up to `1 + policy.max_retries` times, sleeping with backoff between
/// transient failures.
pub(crate) fn run_with_retry<T, E>(
    policy: &RetryPolicy,
    mut f: impl FnMut() -> Attempt<T, E>,
) -> Result<T, E> {
    let mut attempt = 0u32;
    loop {
        match f() {
            Attempt::Ok(v) => return Ok(v),
            Attempt::Fatal(e) => return Err(e),
            Attempt::Transient(e) => {
                if attempt >= policy.max_retries {
                    return Err(e);
                }
                policy.sleep_before_retry(attempt);
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zero_retries_with_standard_backoff() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_retries, 0);
        assert_eq!(p.base_backoff, Duration::from_millis(100));
        assert_eq!(p.max_backoff, Duration::from_secs(2));
    }

    #[test]
    fn http_status_transient_table() {
        for code in [408, 429, 500, 501, 502, 503, 504, 520, 521, 522, 523, 524] {
            assert!(http_status_is_transient(code), "{code} should be transient");
        }
        for code in [
            200, 201, 204, 301, 400, 401, 403, 404, 405, 409, 410, 411, 505, 511,
        ] {
            assert!(
                !http_status_is_transient(code),
                "{code} should NOT be transient"
            );
        }
    }

    #[test]
    fn backoff_zero_base_is_always_zero() {
        let p = RetryPolicy {
            max_retries: 3,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::from_secs(2),
        };
        for i in 0..5 {
            assert_eq!(p.backoff_delay(i), Duration::ZERO);
        }
    }

    #[test]
    fn backoff_respects_max_cap() {
        let p = RetryPolicy {
            max_retries: 10,
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(250),
        };
        // Even with large attempt index, delay ≤ max_backoff.
        for i in 0..8 {
            assert!(p.backoff_delay(i) <= Duration::from_millis(250));
        }
    }

    #[test]
    fn run_with_retry_succeeds_after_transients() {
        let policy = RetryPolicy {
            max_retries: 2,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        };
        let mut n = 0u32;
        let out = run_with_retry(&policy, || {
            n += 1;
            if n < 3 {
                Attempt::Transient("boom")
            } else {
                Attempt::Ok(42)
            }
        });
        assert_eq!(out, Ok(42));
        assert_eq!(n, 3);
    }

    #[test]
    fn run_with_retry_fatal_stops_immediately() {
        let policy = RetryPolicy::new(5);
        let mut n = 0u32;
        let out: Result<(), &str> = run_with_retry(&policy, || {
            n += 1;
            Attempt::Fatal("nope")
        });
        assert_eq!(out, Err("nope"));
        assert_eq!(n, 1);
    }

    #[test]
    fn run_with_retry_zero_budget_single_attempt() {
        let policy = RetryPolicy {
            max_retries: 0,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        };
        let mut n = 0u32;
        let out: Result<(), &str> = run_with_retry(&policy, || {
            n += 1;
            Attempt::Transient("503")
        });
        assert_eq!(out, Err("503"));
        assert_eq!(n, 1);
    }
}
