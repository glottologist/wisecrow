//! Per-user rate limiting for cost-incurring (LLM-backed) server functions, so a
//! single authenticated user cannot exhaust the shared LLM API budget. In-memory
//! and per-process — sufficient for the single-instance calypso deploy.

use std::num::NonZeroU32;
use std::sync::OnceLock;

use dioxus::prelude::ServerFnError;
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};

const DEFAULT_PER_MIN: u32 = 20;

fn llm_limiter() -> &'static DefaultKeyedRateLimiter<i32> {
    static LIMITER: OnceLock<DefaultKeyedRateLimiter<i32>> = OnceLock::new();
    LIMITER.get_or_init(|| {
        let per_min = std::env::var("WISECROW__LLM_RATELIMIT_PER_MIN")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .and_then(NonZeroU32::new)
            .unwrap_or_else(|| NonZeroU32::new(DEFAULT_PER_MIN).expect("nonzero default"));
        RateLimiter::keyed(Quota::per_minute(per_min))
    })
}

/// Checks the per-user LLM quota. Returns an error (429-equivalent) when the user
/// has exceeded their allowance for the current window.
///
/// # Errors
///
/// Returns a `ServerFnError` when the quota is exceeded.
pub fn check_llm_quota(user_id: i32) -> Result<(), ServerFnError> {
    if llm_limiter().check_key(&user_id).is_err() {
        return Err(ServerFnError::new(
            "Rate limit exceeded: too many LLM requests, please wait",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_limit_is_per_user() {
        let limiter: DefaultKeyedRateLimiter<i32> =
            RateLimiter::keyed(Quota::per_minute(NonZeroU32::new(3).expect("nonzero")));
        // One user gets a burst of 3, then is limited.
        assert!(limiter.check_key(&1).is_ok());
        assert!(limiter.check_key(&1).is_ok());
        assert!(limiter.check_key(&1).is_ok());
        assert!(limiter.check_key(&1).is_err());
        // A different user is unaffected.
        assert!(limiter.check_key(&2).is_ok());
    }
}
