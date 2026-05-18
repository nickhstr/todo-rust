//! Per-IP token-bucket rate limit. Cheap, in-memory, sized for the
//! "/login can be hammered" case — not a general DOS shield.
//!
//! Bucket fills at `rate_per_sec`, capped at `burst`. Each request consumes
//! one token; on empty bucket we return 429.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
    rate_per_sec: f64,
    burst: f64,
    pub(crate) trust_forwarded_for: bool,
    enabled: bool,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    /// `burst` tokens initially, refilled at `rate_per_sec` tokens/sec.
    pub fn new(rate_per_sec: f64, burst: f64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            rate_per_sec,
            burst,
            trust_forwarded_for: false,
            enabled: true,
        }
    }

    /// Honor `X-Forwarded-For` (rightmost entry) as the client IP. Only safe
    /// behind a trusted proxy that overwrites the header on inbound.
    #[must_use]
    pub fn trust_forwarded_for(mut self, trust: bool) -> Self {
        self.trust_forwarded_for = trust;
        self
    }

    /// When `false`, every request bypasses the limiter (no bucket lookup, no
    /// metric increment). Default is `true` (limiting active). Set to `false`
    /// only when running load tests, where a single source IP would otherwise
    /// exhaust the burst window immediately.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut guard = self.inner.lock().expect("rate limit mutex poisoned");
        let bucket = guard.entry(ip).or_insert_with(|| Bucket {
            tokens: self.burst,
            last: now,
        });
        // Refill since last hit.
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.rate_per_sec).min(self.burst);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

pub async fn rate_limit_middleware(
    State(limiter): State<RateLimiter>,
    conn: Option<ConnectInfo<std::net::SocketAddr>>,
    req: Request,
    next: Next,
) -> Response {
    if !limiter.enabled {
        return next.run(req).await;
    }
    let peer_ip = conn
        .map(|c| c.0.ip())
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    let ip = if limiter.trust_forwarded_for {
        client_ip_from_headers(req.headers()).unwrap_or(peer_ip)
    } else {
        peer_ip
    };
    if limiter.check(ip) {
        next.run(req).await
    } else {
        // Generic counter — this limiter wraps multiple auth endpoints (login
        // and signup), so attributing rate-limited hits to /login alone via
        // `auth_logins_total{result="rate_limited"}` would be misleading.
        metrics::counter!("auth_rate_limited_total").increment(1);
        (StatusCode::TOO_MANY_REQUESTS, "too many requests").into_response()
    }
}

/// Return the rightmost entry in `X-Forwarded-For` if it parses as an IP.
/// Rightmost-untrusted is the safe default when sitting behind exactly one
/// proxy that overwrites the header; if you have N trusted proxies, you'd
/// instead skip the last N entries.
fn client_ip_from_headers(headers: &HeaderMap) -> Option<IpAddr> {
    let xff = headers.get("x-forwarded-for")?.to_str().ok()?;
    xff.split(',')
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .next_back()
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use axum::http::HeaderMap;

    use super::{client_ip_from_headers, RateLimiter};

    fn ip(b: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, b))
    }

    #[test]
    fn burst_then_block() {
        // Rate = 0 (no refill), burst of 3.
        let rl = RateLimiter::new(0.0, 3.0);
        let a = ip(1);
        assert!(rl.check(a));
        assert!(rl.check(a));
        assert!(rl.check(a));
        assert!(!rl.check(a));
    }

    #[test]
    fn buckets_are_per_ip() {
        let rl = RateLimiter::new(0.0, 1.0);
        let a = ip(1);
        let b = ip(2);
        assert!(rl.check(a));
        assert!(!rl.check(a));
        // b has its own bucket.
        assert!(rl.check(b));
    }

    #[test]
    fn xff_picks_rightmost_ip() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            "203.0.113.7, 198.51.100.4".parse().unwrap(),
        );
        assert_eq!(
            client_ip_from_headers(&h),
            Some("198.51.100.4".parse().unwrap()),
        );
    }

    #[test]
    fn xff_handles_single_value() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "203.0.113.7".parse().unwrap());
        assert_eq!(
            client_ip_from_headers(&h),
            Some("203.0.113.7".parse().unwrap()),
        );
    }

    #[test]
    fn xff_returns_none_when_absent_or_unparseable() {
        let h = HeaderMap::new();
        assert_eq!(client_ip_from_headers(&h), None);
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "not-an-ip".parse().unwrap());
        assert_eq!(client_ip_from_headers(&h), None);
    }

    #[tokio::test]
    async fn disabled_bypass_lets_all_requests_through() {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
            middleware::from_fn_with_state,
            routing::get,
            Router,
        };
        use tower::ServiceExt;

        // Burst=1 and rate=0: without bypass, request 2+ would 429.
        let limiter = RateLimiter::new(0.0, 1.0).enabled(false);
        let app = Router::new()
            .route("/x", get(|| async { "ok" }))
            .route_layer(from_fn_with_state(limiter, super::rate_limit_middleware));

        for _ in 0..5 {
            let req = Request::builder().uri("/x").body(Body::empty()).unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn enabled_limiter_still_429s_when_bucket_empty() {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
            middleware::from_fn_with_state,
            routing::get,
            Router,
        };
        use tower::ServiceExt;

        // Default `enabled=true`; burst=1 so 2nd request must 429.
        let limiter = RateLimiter::new(0.0, 1.0);
        let app = Router::new()
            .route("/x", get(|| async { "ok" }))
            .route_layer(from_fn_with_state(limiter, super::rate_limit_middleware));

        let req = Request::builder().uri("/x").body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = Request::builder().uri("/x").body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
