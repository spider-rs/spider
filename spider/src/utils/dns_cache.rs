use dashmap::DashMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Maximum number of cached entries before eviction on overflow.
const MAX_ENTRIES: usize = 5_000;

/// Cached DNS entry with TTL tracking.
pub(crate) struct DnsEntry {
    /// Pre-computed socket addresses (port 0) — avoids per-hit IpAddr→SocketAddr conversion.
    sockaddrs: Arc<[SocketAddr]>,
    /// Original IP addresses for the public `resolve()` API.
    addrs: Vec<IpAddr>,
    expires: Instant,
}

/// Thread-safe DNS resolution cache with configurable TTL.
///
/// Resolves hostnames via the system resolver and caches results
/// for up to `ttl`. Expired entries are served as misses and
/// re-resolved on next access. Capped at [`MAX_ENTRIES`]; overflow
/// triggers expired-entry eviction followed by LRU-style trimming.
pub struct DnsCache {
    pub(crate) cache: DashMap<String, DnsEntry>,
    pub(crate) ttl: Duration,
}

impl DnsCache {
    /// Create a new DNS cache with the given TTL for entries.
    pub fn new(ttl: Duration) -> Self {
        Self {
            cache: DashMap::with_capacity(128),
            ttl,
        }
    }

    /// Resolve a hostname, returning cached results if available and not expired.
    ///
    /// On cache miss or expiry the system resolver is called on a blocking
    /// thread so the async runtime is never stalled.
    pub async fn resolve(&self, host: &str) -> Option<Vec<IpAddr>> {
        // Check cache first.
        if let Some(entry) = self.cache.get(host) {
            if entry.expires > Instant::now() {
                return Some(entry.addrs.clone());
            }
        }

        // Cache miss or expired — resolve via system resolver on a blocking thread.
        let host_owned = format!("{}:0", host);
        let result = tokio::task::spawn_blocking(move || {
            host_owned
                .to_socket_addrs()
                .ok()
                .map(|addrs| addrs.collect::<Vec<SocketAddr>>())
        })
        .await
        .ok()
        .flatten();

        match result {
            Some(resolved) if !resolved.is_empty() => {
                let ips: Vec<IpAddr> = resolved.iter().map(|s| s.ip()).collect();
                // Evict overflow before inserting.
                if self.cache.len() >= MAX_ENTRIES {
                    self.evict_expired();
                    // If still over capacity after eviction, remove oldest entries.
                    if self.cache.len() >= MAX_ENTRIES {
                        let to_remove = MAX_ENTRIES / 10;
                        let keys_to_remove: Vec<String> = self
                            .cache
                            .iter()
                            .take(to_remove)
                            .map(|entry| entry.key().clone())
                            .collect();
                        for k in keys_to_remove {
                            self.cache.remove(&k);
                        }
                    }
                }

                self.cache.insert(
                    host.to_string(),
                    DnsEntry {
                        sockaddrs: resolved.into(),
                        addrs: ips.clone(),
                        expires: Instant::now() + self.ttl,
                    },
                );
                Some(ips)
            }
            _ => None,
        }
    }

    /// Batch pre-resolve hostnames concurrently. Best-effort — errors are
    /// silently ignored.
    pub async fn pre_resolve(&self, hosts: &[&str]) {
        let mut set = tokio::task::JoinSet::new();
        for &host in hosts {
            let host_string = host.to_string();
            let ttl = self.ttl;
            // We cannot borrow `self` across spawn, so resolve inline after join.
            set.spawn(async move {
                let addr_str = format!("{}:0", host_string);
                let result = tokio::task::spawn_blocking(move || {
                    addr_str
                        .to_socket_addrs()
                        .ok()
                        .map(|addrs| addrs.collect::<Vec<SocketAddr>>())
                })
                .await
                .ok()
                .flatten();
                (host_string, result, ttl)
            });
        }
        while let Some(Ok((host, result, ttl))) = set.join_next().await {
            if let Some(resolved) = result {
                if !resolved.is_empty() {
                    let ips: Vec<IpAddr> = resolved.iter().map(|s| s.ip()).collect();
                    self.cache.insert(
                        host,
                        DnsEntry {
                            sockaddrs: resolved.into(),
                            addrs: ips,
                            expires: Instant::now() + ttl,
                        },
                    );
                }
            }
        }
    }

    /// Number of cached entries (including potentially expired ones).
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns `true` if the cache contains no entries.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Remove all expired entries from the cache.
    pub fn evict_expired(&self) {
        let now = Instant::now();
        self.cache.retain(|_, v| v.expires > now);
    }
}

/// Wrapper that holds an `Arc<DnsCache>` so it can be moved into async
/// futures returned by [`Resolve::resolve`].
pub struct DnsCacheResolver(pub Arc<DnsCache>);

impl crate::client::dns::Resolve for DnsCacheResolver {
    fn resolve(&self, name: crate::client::dns::Name) -> crate::client::dns::Resolving {
        let host = name.as_str().to_string();
        let cache = self.0.clone();

        Box::pin(async move {
            let now = Instant::now();

            // Fast path: cache hit — Arc clone only, zero alloc for the address list.
            if let Some(entry) = cache.cache.get(&host) {
                if entry.expires > now {
                    let addrs = entry.sockaddrs.clone();
                    let iter: crate::client::dns::Addrs = Box::new(ArcSocketAddrIter {
                        inner: addrs,
                        pos: 0,
                    });
                    return Ok(iter);
                }
            }

            // Cache miss — resolve on blocking thread.
            let host_for_resolve = format!("{}:0", host);
            let result = tokio::task::spawn_blocking(move || {
                host_for_resolve
                    .to_socket_addrs()
                    .ok()
                    .map(|addrs| addrs.collect::<Vec<SocketAddr>>())
            })
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?
            .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                "dns resolution failed".into()
            })?;

            // Store in cache.
            let ips: Vec<IpAddr> = result.iter().map(|s| s.ip()).collect();
            let sockaddrs: Arc<[SocketAddr]> = result.into();
            if !ips.is_empty() {
                cache.cache.insert(
                    host,
                    DnsEntry {
                        sockaddrs: sockaddrs.clone(),
                        addrs: ips,
                        expires: Instant::now() + cache.ttl,
                    },
                );
            }

            let iter: crate::client::dns::Addrs = Box::new(ArcSocketAddrIter {
                inner: sockaddrs,
                pos: 0,
            });
            Ok(iter)
        })
    }
}

/// Iterator over `Arc<[SocketAddr]>` that avoids cloning the Vec on every cache hit.
struct ArcSocketAddrIter {
    inner: Arc<[SocketAddr]>,
    pos: usize,
}

impl Iterator for ArcSocketAddrIter {
    type Item = SocketAddr;
    fn next(&mut self) -> Option<SocketAddr> {
        if self.pos < self.inner.len() {
            let addr = self.inner[self.pos];
            self.pos += 1;
            Some(addr)
        } else {
            None
        }
    }
}

/// Global shared DNS cache with 5-minute TTL, wrapped for reqwest.
pub fn shared_dns_cache() -> Arc<DnsCacheResolver> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Arc<DnsCacheResolver>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            Arc::new(DnsCacheResolver(Arc::new(DnsCache::new(
                Duration::from_secs(300),
            ))))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolve_localhost_returns_result() {
        let cache = DnsCache::new(Duration::from_secs(60));
        let result = cache.resolve("localhost").await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cache_hit_returns_same_result() {
        let cache = DnsCache::new(Duration::from_secs(60));
        let first = cache.resolve("localhost").await;
        assert_eq!(cache.len(), 1);
        let second = cache.resolve("localhost").await;
        assert_eq!(first, second);
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn expired_entry_triggers_re_resolve() {
        let cache = DnsCache::new(Duration::from_millis(1));
        let _ = cache.resolve("localhost").await;
        assert_eq!(cache.len(), 1);

        // Wait for expiry.
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Should still resolve (re-fetches), cache still has 1 entry.
        let result = cache.resolve("localhost").await;
        assert!(result.is_some());
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn unknown_host_returns_none() {
        let cache = DnsCache::new(Duration::from_secs(60));
        let result = cache
            .resolve("this.host.definitely.does.not.exist.example")
            .await;
        assert!(result.is_none());
        assert!(cache.is_empty());
    }

    #[tokio::test]
    async fn pre_resolve_populates_cache() {
        let cache = DnsCache::new(Duration::from_secs(60));
        cache.pre_resolve(&["localhost"]).await;
        assert!(cache.len() >= 1);
        // Subsequent resolve should be a cache hit.
        let result = cache.resolve("localhost").await;
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn evict_expired_removes_stale_entries() {
        let cache = DnsCache::new(Duration::from_millis(1));
        let _ = cache.resolve("localhost").await;
        assert_eq!(cache.len(), 1);

        tokio::time::sleep(Duration::from_millis(10)).await;
        cache.evict_expired();
        assert!(cache.is_empty());
    }

    #[test]
    fn new_cache_is_empty() {
        let cache = DnsCache::new(Duration::from_secs(60));
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[tokio::test]
    async fn resolver_cache_hit_returns_socket_addrs() {
        let dns = Arc::new(DnsCache::new(Duration::from_secs(60)));
        // Prime the cache.
        let _ = dns.resolve("localhost").await;
        assert_eq!(dns.len(), 1);

        let resolver = DnsCacheResolver(dns);
        let name = "localhost".parse().expect("valid name");
        let addrs: Vec<SocketAddr> = crate::client::dns::Resolve::resolve(&resolver, name)
            .await
            .expect("should resolve")
            .collect();
        assert!(!addrs.is_empty());
    }
}
