use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use actix_web::HttpRequest;

/// Sliding-window limiter kept in process memory; limits apply per backend instance.
pub struct RateLimiter<K = IpAddr> {
    limit: usize,
    window: Duration,
    state: Mutex<LimiterState<K>>,
}

struct LimiterState<K> {
    hits: HashMap<K, VecDeque<Instant>>,
    last_sweep: Instant,
}

impl<K: Eq + Hash + Clone> RateLimiter<K> {
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit,
            window,
            state: Mutex::new(LimiterState {
                hits: HashMap::new(),
                last_sweep: Instant::now(),
            }),
        }
    }

    /// Records a hit for `key`, or returns how long to wait when the limit is reached.
    pub fn check(&self, key: &K) -> Result<(), Duration> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: &K, now: Instant) -> Result<(), Duration> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let window = self.window;
        let expired = |hit: &Instant| now.saturating_duration_since(*hit) >= window;

        if now.saturating_duration_since(state.last_sweep) >= window {
            state.hits.retain(|_, hits| {
                while hits.front().is_some_and(expired) {
                    hits.pop_front();
                }
                !hits.is_empty()
            });
            state.last_sweep = now;
        }

        let hits = state.hits.entry(key.clone()).or_default();
        while hits.front().is_some_and(expired) {
            hits.pop_front();
        }
        if hits.len() >= self.limit {
            let oldest = *hits.front().expect("limit is at least one hit");
            return Err(window.saturating_sub(now.saturating_duration_since(oldest)));
        }
        hits.push_back(now);
        Ok(())
    }
}

/// Uses `X-Real-IP` only when `TRUST_PROXY_HEADERS=true`, i.e. when the backend is
/// reachable exclusively through a proxy that overwrites that header.
pub fn client_ip(request: &HttpRequest) -> Option<IpAddr> {
    let trust_proxy = std::env::var("TRUST_PROXY_HEADERS")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if trust_proxy {
        if let Some(ip) = request
            .headers()
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok())
        {
            return Some(ip);
        }
    }
    request.peer_addr().map(|address| address.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_hits_per_key_within_the_window() {
        let limiter = RateLimiter::<u8>::new(2, Duration::from_secs(60));
        let start = Instant::now();
        assert!(limiter.check_at(&1, start).is_ok());
        assert!(limiter
            .check_at(&1, start + Duration::from_secs(10))
            .is_ok());
        assert_eq!(
            limiter.check_at(&1, start + Duration::from_secs(20)),
            Err(Duration::from_secs(40))
        );
        assert!(limiter
            .check_at(&2, start + Duration::from_secs(20))
            .is_ok());
        assert!(limiter
            .check_at(&1, start + Duration::from_secs(60))
            .is_ok());
        assert!(limiter
            .check_at(&1, start + Duration::from_secs(65))
            .is_err());
    }

    #[test]
    fn sweeps_idle_keys() {
        let limiter = RateLimiter::<u8>::new(1, Duration::from_secs(60));
        let start = Instant::now();
        limiter.check_at(&1, start).unwrap();
        limiter
            .check_at(&2, start + Duration::from_secs(120))
            .unwrap();
        let state = limiter.state.lock().unwrap();
        assert!(!state.hits.contains_key(&1));
        assert!(state.hits.contains_key(&2));
    }
}
