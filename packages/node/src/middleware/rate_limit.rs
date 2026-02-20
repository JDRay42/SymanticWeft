//! Per-IP fixed-window rate limiting middleware (ADR-0006).
//!
//! Applies a configurable per-minute request cap keyed on the client IP,
//! extracted from `X-Forwarded-For` → `X-Real-IP` → `"unknown"` in that order.
//!
//! When the limit is exceeded the middleware returns HTTP 429 with a
//! `Retry-After` header whose value is the number of seconds until the
//! current window resets.
//!
//! A limit of `0` disables rate limiting entirely (useful in tests or for
//! trusted internal deployments).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::Request,
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use semanticweft_node_api::ErrorResponse;

// ---------------------------------------------------------------------------
// RateLimiter
// ---------------------------------------------------------------------------

/// Fixed-window per-IP rate limiter.
///
/// Each unique IP key gets an independent token bucket that resets every
/// `window` duration. Thread-safe; cheaply cloneable via `Arc`.
pub struct RateLimiter {
    state: RwLock<HashMap<String, Bucket>>,
    max_per_window: u32,
    window: Duration,
}

struct Bucket {
    count: u32,
    window_start: Instant,
}

impl RateLimiter {
    /// Create a new rate limiter with the given per-minute limit.
    ///
    /// Pass `0` to disable rate limiting.
    pub fn new(max_per_minute: u32) -> Self {
        Self {
            state: RwLock::new(HashMap::new()),
            max_per_window: max_per_minute,
            window: Duration::from_secs(60),
        }
    }

    /// Check whether a request from `ip_key` is within the limit.
    ///
    /// Returns `Ok(())` if the request is allowed, or `Err(retry_after_secs)`
    /// if the limit is exceeded.
    pub fn check(&self, ip_key: &str) -> Result<(), u64> {
        if self.max_per_window == 0 {
            return Ok(());
        }

        let now = Instant::now();
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());

        let bucket = state.entry(ip_key.to_string()).or_insert_with(|| Bucket {
            count: 0,
            window_start: now,
        });

        let elapsed = now.duration_since(bucket.window_start);

        if elapsed >= self.window {
            // Window has expired — start a fresh window.
            bucket.count = 1;
            bucket.window_start = now;
            return Ok(());
        }

        if bucket.count >= self.max_per_window {
            let retry_after = (self.window.saturating_sub(elapsed)).as_secs().max(1);
            return Err(retry_after);
        }

        bucket.count += 1;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Middleware function
// ---------------------------------------------------------------------------

/// Axum `from_fn` middleware that enforces per-IP rate limiting.
pub async fn rate_limit_middleware(
    limiter: Arc<RateLimiter>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip_key = extract_ip(&req);

    match limiter.check(&ip_key) {
        Ok(()) => next.run(req).await,
        Err(retry_after) => {
            let body = ErrorResponse::new("rate_limit_exceeded", "rate limit exceeded");
            let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
            if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
                resp.headers_mut().insert("retry-after", v);
            }
            resp
        }
    }
}

// ---------------------------------------------------------------------------
// IP extraction
// ---------------------------------------------------------------------------

/// Extract the client IP from common proxy headers, falling back to `"unknown"`.
fn extract_ip(req: &Request<Body>) -> String {
    // X-Forwarded-For: client, proxy1, proxy2  — leftmost is the real client.
    if let Some(xff) = req.headers().get("x-forwarded-for") {
        if let Ok(s) = xff.to_str() {
            if let Some(ip) = s.split(',').next().map(str::trim) {
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        }
    }

    // X-Real-IP: set by nginx/Caddy.
    if let Some(xri) = req.headers().get("x-real-ip") {
        if let Ok(s) = xri.to_str() {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }

    // No identifiable IP — apply a shared "unknown" bucket.
    "unknown".to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_limit_always_passes() {
        let rl = RateLimiter::new(0);
        for _ in 0..1000 {
            assert!(rl.check("1.2.3.4").is_ok());
        }
    }

    #[test]
    fn within_limit_passes() {
        let rl = RateLimiter::new(5);
        for _ in 0..5 {
            assert!(rl.check("1.2.3.4").is_ok());
        }
    }

    #[test]
    fn exceeding_limit_returns_retry_after() {
        let rl = RateLimiter::new(3);
        for _ in 0..3 {
            assert!(rl.check("1.2.3.4").is_ok());
        }
        let err = rl.check("1.2.3.4").unwrap_err();
        assert!(err > 0 && err <= 60, "retry-after should be 1–60s, got {err}");
    }

    #[test]
    fn different_ips_have_independent_buckets() {
        let rl = RateLimiter::new(1);
        assert!(rl.check("1.1.1.1").is_ok());
        assert!(rl.check("2.2.2.2").is_ok()); // different IP, own bucket
        assert!(rl.check("1.1.1.1").is_err()); // 1.1.1.1 is now exhausted
    }
}
