//! Rate limits per client address: publishes, sign-ins, other changes and
//! reads each have a token bucket per IP. Behind a proxy, the client's
//! address comes from `X-Forwarded-For`, read only when the connection
//! comes from a trusted proxy. Over the limit is 429 with `Retry-After`.

use crate::api::ApiError;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `count` requests per `per`, refilled evenly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub count: u32,
    pub per: Duration,
}

impl Rate {
    /// `30/hour`, `10/minute`, `5/second`, `100/day`, `20/90` (seconds), or
    /// `off` (`None`).
    pub fn parse(text: &str) -> Result<Option<Rate>, String> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("off") {
            return Ok(None);
        }
        let bad = || format!("`{text}` is not a rate: write `30/hour`, `10/minute` or `off`");
        let (count, per) = text.split_once('/').ok_or_else(bad)?;
        let count: u32 = count.trim().parse().map_err(|_| bad())?;
        let seconds = match per.trim() {
            "s" | "second" | "sec" => 1,
            "m" | "minute" | "min" => 60,
            "h" | "hour" => 3600,
            "d" | "day" => 86_400,
            n => n.parse::<u64>().map_err(|_| bad())?,
        };
        if count == 0 || seconds == 0 {
            return Err(bad());
        }
        Ok(Some(Rate {
            count,
            per: Duration::from_secs(seconds),
        }))
    }
}

/// An address or a network (`10.0.0.0/8`, `fd00::/8`, `127.0.0.1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Net {
    addr: IpAddr,
    prefix: u8,
}

impl Net {
    pub fn parse(text: &str) -> Result<Net, String> {
        let text = text.trim();
        let (addr, prefix) = match text.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (text, None),
        };
        let addr: IpAddr = addr
            .parse()
            .map_err(|_| format!("`{text}` is not an address or network"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("`{text}` has no valid prefix length"))?,
            None => max,
        };
        Ok(Net { addr, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
            v4 => v4,
        };
        let bits = |ip: IpAddr| -> u128 {
            match ip {
                IpAddr::V4(v4) => u128::from(u32::from(v4)) << 96,
                IpAddr::V6(v6) => u128::from(v6),
            }
        };
        if ip.is_ipv4() != self.addr.is_ipv4() {
            return false;
        }
        if self.prefix == 0 {
            return true;
        }
        let mask = u128::MAX << (128 - u32::from(self.prefix));
        bits(ip) & mask == bits(self.addr) & mask
    }
}

/// What a request counts against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Publish,
    Login,
    Write,
    Read,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::Publish => "publish",
            Class::Login => "sign-in",
            Class::Write => "change",
            Class::Read => "read",
        }
    }
}

/// The limits (from `WOWLUA_REGISTRY_RATE_*` and
/// `WOWLUA_REGISTRY_TRUSTED_PROXIES`).
#[derive(Clone, Debug)]
pub struct Limits {
    pub publish: Option<Rate>,
    pub login: Option<Rate>,
    pub write: Option<Rate>,
    pub read: Option<Rate>,
    /// Proxies whose `X-Forwarded-For` names the client.
    pub trusted_proxies: Vec<Net>,
}

impl Default for Limits {
    fn default() -> Self {
        let rate = |count, secs| {
            Some(Rate {
                count,
                per: Duration::from_secs(secs),
            })
        };
        Limits {
            publish: rate(30, 3600),
            login: rate(10, 60),
            write: rate(120, 3600),
            read: rate(600, 60),
            trusted_proxies: Vec::new(),
        }
    }
}

impl Limits {
    fn rate(&self, class: Class) -> Option<Rate> {
        match class {
            Class::Publish => self.publish,
            Class::Login => self.login,
            Class::Write => self.write,
            Class::Read => self.read,
        }
    }

    fn trusted(&self, ip: IpAddr) -> bool {
        self.trusted_proxies.iter().any(|n| n.contains(ip))
    }

    /// The client's address: the connection's, or, from a trusted proxy,
    /// the last address of `X-Forwarded-For` that is no trusted proxy.
    pub fn client(&self, peer: IpAddr, forwarded: Option<&str>) -> IpAddr {
        if !self.trusted(peer) {
            return peer;
        }
        let Some(forwarded) = forwarded else {
            return peer;
        };
        let mut client = peer;
        for hop in forwarded.rsplit(',') {
            let Ok(ip) = hop.trim().parse::<IpAddr>() else {
                break;
            };
            client = ip;
            if !self.trusted(ip) {
                break;
            }
        }
        client
    }
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

/// The buckets of every class and address.
#[derive(Default)]
pub struct Limiter {
    buckets: Mutex<HashMap<(Class, IpAddr), Bucket>>,
}

impl Limiter {
    /// Takes one request from the bucket; the wait until the next one when
    /// it is empty.
    pub fn take(&self, class: Class, ip: IpAddr, rate: Rate) -> Result<(), Duration> {
        let now = Instant::now();
        let capacity = f64::from(rate.count);
        let per_second = capacity / rate.per.as_secs_f64();
        let mut buckets = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        if buckets.len() > 50_000 {
            // Buckets that are full again hold nothing worth keeping.
            buckets.retain(|_, b| {
                b.tokens + now.duration_since(b.at).as_secs_f64() * per_second < capacity
            });
        }
        let bucket = buckets.entry((class, ip)).or_insert(Bucket {
            tokens: capacity,
            at: now,
        });
        let refilled = (bucket.tokens + now.duration_since(bucket.at).as_secs_f64() * per_second)
            .min(capacity);
        bucket.at = now;
        if refilled >= 1.0 {
            bucket.tokens = refilled - 1.0;
            Ok(())
        } else {
            bucket.tokens = refilled;
            Err(Duration::from_secs_f64((1.0 - refilled) / per_second))
        }
    }
}

/// The class of a request; `None` for the health check.
pub fn class_of(method: &Method, path: &str) -> Option<Class> {
    if path == "/health" {
        return None;
    }
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    if *method == Method::PUT
        && segments.len() == 4
        && segments[0] == "v1"
        && segments[1] == "packages"
        && segments[3] != "owners"
    {
        return Some(Class::Publish);
    }
    if path == "/login/github" || path == "/login/callback" {
        return Some(Class::Login);
    }
    if *method == Method::GET || *method == Method::HEAD {
        Some(Class::Read)
    } else {
        Some(Class::Write)
    }
}

/// The middleware.
pub async fn limit(State(app): State<crate::Shared>, request: Request, next: Next) -> Response {
    let Some(class) = class_of(request.method(), request.uri().path()) else {
        return next.run(request).await;
    };
    let limits = &app.config.limits;
    let Some(rate) = limits.rate(class) else {
        return next.run(request).await;
    };
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    let Some(peer) = peer else {
        return next.run(request).await;
    };
    let forwarded: Vec<&str> = request
        .headers()
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();
    let forwarded = (!forwarded.is_empty()).then(|| forwarded.join(","));
    let client = limits.client(peer, forwarded.as_deref());
    match app.limiter.take(class, client, rate) {
        Ok(()) => next.run(request).await,
        Err(wait) => {
            let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
            let mut response = ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate-limited",
                format!(
                    "too many {} requests from this address; try again in {secs} s",
                    class.name()
                ),
            )
            .into_response();
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, v);
            }
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_parse() {
        assert_eq!(
            Rate::parse("30/hour").unwrap(),
            Some(Rate {
                count: 30,
                per: Duration::from_secs(3600)
            })
        );
        assert_eq!(Rate::parse("off").unwrap(), None);
        assert_eq!(
            Rate::parse("5/90").unwrap().unwrap().per,
            Duration::from_secs(90)
        );
        assert!(Rate::parse("0/hour").is_err());
        assert!(Rate::parse("ten").is_err());
    }

    #[test]
    fn networks_contain_addresses() {
        let net = Net::parse("172.16.0.0/12").unwrap();
        assert!(net.contains("172.20.0.5".parse().unwrap()));
        assert!(!net.contains("172.32.0.1".parse().unwrap()));
        assert!(net.contains("::ffff:172.20.0.5".parse().unwrap()));
        assert!(Net::parse("::1").unwrap().contains("::1".parse().unwrap()));
        assert!(Net::parse("10.0.0.0/33").is_err());
    }

    #[test]
    fn forwarded_for_is_read_only_from_trusted_proxies() {
        let limits = Limits {
            trusted_proxies: vec![Net::parse("10.0.0.0/8").unwrap()],
            ..Limits::default()
        };
        let proxy: IpAddr = "10.0.0.2".parse().unwrap();
        let other: IpAddr = "198.51.100.1".parse().unwrap();
        let client: IpAddr = "203.0.113.5".parse().unwrap();
        assert_eq!(limits.client(proxy, Some("203.0.113.5")), client);
        // A client's own forged entry stays left of what the proxy added.
        assert_eq!(limits.client(proxy, Some("1.2.3.4, 203.0.113.5")), client);
        assert_eq!(limits.client(proxy, Some("203.0.113.5, 10.0.0.9")), client);
        assert_eq!(limits.client(other, Some("203.0.113.5")), other);
        assert_eq!(limits.client(proxy, None), proxy);
    }

    #[test]
    fn buckets_refill() {
        let limiter = Limiter::default();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        let rate = Rate {
            count: 2,
            per: Duration::from_secs(3600),
        };
        assert!(limiter.take(Class::Publish, ip, rate).is_ok());
        assert!(limiter.take(Class::Publish, ip, rate).is_ok());
        let wait = limiter.take(Class::Publish, ip, rate).unwrap_err();
        assert!(wait > Duration::from_secs(1700) && wait <= Duration::from_secs(1800));
        // Another class and another address have their own buckets.
        assert!(limiter.take(Class::Read, ip, rate).is_ok());
        assert!(
            limiter
                .take(Class::Publish, "::1".parse().unwrap(), rate)
                .is_ok()
        );
    }

    #[test]
    fn requests_have_classes() {
        assert_eq!(
            class_of(&Method::PUT, "/v1/packages/a/1.0.0"),
            Some(Class::Publish)
        );
        assert_eq!(
            class_of(&Method::PUT, "/v1/packages/a/owners"),
            Some(Class::Write)
        );
        assert_eq!(
            class_of(&Method::PUT, "/v1/packages/a/1.0.0/unyank"),
            Some(Class::Write)
        );
        assert_eq!(class_of(&Method::GET, "/login/github"), Some(Class::Login));
        assert_eq!(class_of(&Method::GET, "/index/a"), Some(Class::Read));
        assert_eq!(
            class_of(&Method::POST, "/account/tokens"),
            Some(Class::Write)
        );
        assert_eq!(class_of(&Method::GET, "/health"), None);
    }
}
