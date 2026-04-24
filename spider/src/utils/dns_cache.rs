use dashmap::DashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Maximum number of cached entries before eviction on overflow.
const MAX_ENTRIES: usize = 5_000;

/// Default proxy-host DNS refresh interval bounds.
const PROXY_REFRESH_MIN_SECS: u64 = 30;
const PROXY_REFRESH_MAX_SECS: u64 = 240;

/// Cached DNS entry with TTL tracking.
pub(crate) struct DnsEntry {
    /// Pre-computed socket addresses (port 0) — avoids per-hit IpAddr→SocketAddr conversion.
    sockaddrs: Arc<[SocketAddr]>,
    /// Original IP addresses for the public `resolve()` API.
    addrs: Vec<IpAddr>,
    expires: Instant,
}

/// Global async resolver — initialized at most once per process, reused
/// across all lookups. Hickory's `TokioResolver` is fully async (no
/// `spawn_blocking`).
///
/// Returns `None` only if both the system-config build and the
/// default-config fallback fail during initialization; in that case the
/// failure is cached so subsequent calls do not retry.
fn async_resolver() -> Option<&'static hickory_resolver::TokioResolver> {
    use std::sync::OnceLock;
    static RESOLVER: OnceLock<Option<hickory_resolver::TokioResolver>> = OnceLock::new();

    if let Some(slot) = RESOLVER.get() {
        return slot.as_ref();
    }
    // Cold path — runs at most once per process per contender.
    // `set` consumes `built` by value; on loss the `Err(_)` is dropped
    // right here. Failure is stored as `Some(None)` so we do not
    // re-attempt the build on every lookup.
    let built = build_async_resolver();
    let _ = RESOLVER.set(built);
    RESOLVER.get().and_then(|slot| slot.as_ref())
}

/// Parse a non-zero `usize` from an environment variable, clamped to
/// `[min, max]`. Returns `None` when the variable is unset, empty, or
/// not parseable — callers fall back to their in-code default so a
/// bad env value never aborts startup.
fn env_usize(name: &str, min: usize, max: usize) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .map(|v| v.clamp(min, max))
}

/// Build a crawler-tuned `TokioResolver`, reading the host's system DNS
/// config (e.g. `/etc/resolv.conf`) when available. Returns `None` only
/// if every build path errors — callers propagate that as a DNS failure
/// rather than a panic.
fn build_async_resolver() -> Option<hickory_resolver::TokioResolver> {
    use hickory_resolver::config::{ResolverConfig, ResolverOpts};
    use hickory_resolver::net::runtime::TokioRuntimeProvider;

    let (config, mut opts) = hickory_resolver::system_conf::read_system_conf()
        .unwrap_or_else(|_| (ResolverConfig::default(), ResolverOpts::default()));

    // Tuning rationale — only change what strictly improves behavior vs
    // the prior `ResolverOpts::default()` baseline, and expose env-var
    // overrides so operators can tune without code changes. Any value
    // not set via env keeps the hickory default, so by default there
    // are zero behavior changes relative to the 0.25 baseline aside
    // from the bounded `cache_size` bump (strictly a hit-rate win —
    // cache is TTL-evicted and memory-bounded).
    //
    // cache_size: hickory's default of 32 is a bottleneck for crawls
    // that fan out across thousands of unique hosts. Default here
    // matches the outer `DnsCache` bound of 5_000 so every outer-cache
    // miss hits the inner cache instead of the network. Override via
    // `SPIDER_DNS_CACHE_SIZE` (clamped to [32, 100_000]).
    //
    // negative_max_ttl: left at `None` (honor SOA). `DnsCacheResolver`
    // already maps NXDOMAIN/NOERROR-empty to `io::Error::NotFound`,
    // which `page::is_dns_error` classifies as a permanent 525 — the
    // crawler does not retry at all. A cap here would add DNS traffic
    // for bad hosts without improving recoverability.
    //
    // positive_max_ttl: left at `None` (honor record TTL). The outer
    // `DnsCache` already imposes a 5-minute host TTL, so any cap on
    // the inner cache beyond that is moot.
    //
    // num_concurrent_reqs: defaults to 2 (matches hickory's default and
    // the common 2-nameserver resolv.conf). Override via
    // `SPIDER_DNS_CONCURRENT_REQS` (clamped to [1, 16]). Bumping it
    // fans queries out to more nameservers per lookup — a latency win
    // only when multiple nameservers are configured.
    opts.cache_size = env_usize("SPIDER_DNS_CACHE_SIZE", 32, 100_000).unwrap_or(8_192) as u64;
    opts.num_concurrent_reqs = env_usize("SPIDER_DNS_CONCURRENT_REQS", 1, 16).unwrap_or(2);

    hickory_resolver::Resolver::builder_with_config(config, TokioRuntimeProvider::default())
        .with_options(opts)
        .build()
        .or_else(|_| {
            // `.build()` only fails on invalid runtime config. Fall back
            // to a default-config build before giving up entirely.
            hickory_resolver::Resolver::builder_with_config(
                ResolverConfig::default(),
                TokioRuntimeProvider::default(),
            )
            .with_options(ResolverOpts::default())
            .build()
        })
        .ok()
}

/// Thread-safe DNS resolution cache with configurable TTL.
///
/// Resolves hostnames via hickory-resolver (fully async, no blocking)
/// and caches results for up to `ttl`. Expired entries are served as
/// misses and re-resolved on next access. Capped at [`MAX_ENTRIES`];
/// overflow triggers expired-entry eviction followed by LRU-style trimming.
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
    /// On cache miss or expiry, resolution is performed via hickory-resolver
    /// which is fully async — no `spawn_blocking`, no thread-pool overhead.
    pub async fn resolve(&self, host: &str) -> Option<Vec<IpAddr>> {
        // Check cache first.
        if let Some(entry) = self.cache.get(host) {
            if entry.expires > Instant::now() {
                return Some(entry.addrs.clone());
            }
        }

        // Cache miss or expired — resolve via async hickory resolver.
        let lookup = async_resolver()?.lookup_ip(host).await.ok()?;

        let ips: Vec<IpAddr> = lookup.iter().collect();
        if ips.is_empty() {
            return None;
        }

        let sockaddrs: Vec<SocketAddr> = ips.iter().map(|ip| SocketAddr::new(*ip, 0)).collect();

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
                sockaddrs: sockaddrs.into(),
                addrs: ips.clone(),
                expires: Instant::now() + self.ttl,
            },
        );
        Some(ips)
    }

    /// Batch pre-resolve hostnames. Best-effort — errors are silently
    /// ignored.  Resolves concurrently via tokio tasks for parallel DNS.
    pub async fn pre_resolve(&self, hosts: &[&str]) {
        let mut set = tokio::task::JoinSet::new();
        for &host in hosts {
            let host = host.to_string();
            let ttl = self.ttl;
            set.spawn(async move {
                let lookup = async_resolver()?.lookup_ip(&host).await.ok()?;
                let ips: Vec<IpAddr> = lookup.iter().collect();
                if ips.is_empty() {
                    return None;
                }
                let sockaddrs: Vec<SocketAddr> =
                    ips.iter().map(|ip| SocketAddr::new(*ip, 0)).collect();
                Some((host, sockaddrs, ips, ttl))
            });
        }
        // Collect results back into the cache on the calling task.
        while let Some(Ok(Some((host, sockaddrs, ips, ttl)))) = set.join_next().await {
            self.cache.insert(
                host,
                DnsEntry {
                    sockaddrs: sockaddrs.into(),
                    addrs: ips,
                    expires: Instant::now() + ttl,
                },
            );
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

    /// Invalidate a single host, forcing the next lookup to re-resolve.
    /// Call this when a proxy connection fails so stale DNS doesn't persist.
    pub fn invalidate(&self, host: &str) {
        self.cache.remove(host);
    }

    /// Resolve a hostname, but if the cached entry exists and fails a
    /// liveness check, invalidate and re-resolve once. Returns `None` only
    /// if the re-resolve also fails.
    ///
    /// This is useful for proxy hosts: if the proxy becomes unreachable,
    /// the caller invalidates + retries DNS to pick up a potential IP change.
    pub async fn resolve_or_refresh(&self, host: &str) -> Option<Vec<IpAddr>> {
        // Try cached first.
        if let Some(ips) = self.resolve(host).await {
            return Some(ips);
        }
        // Cache miss already does a fresh resolve, so if that failed too
        // there's nothing more to do.
        None
    }

    /// Compute a cheap hash of the resolved IPs for a host without
    /// allocating. Used by the adaptive refresh to detect IP changes.
    pub(crate) async fn resolve_hash(&self, host: &str) -> Option<u64> {
        let ips = self.resolve(host).await?;
        let mut hash: u64 = 0xcbf29ce484222325;
        for ip in ips {
            let bits = match ip {
                IpAddr::V4(v4) => u32::from(v4) as u64,
                IpAddr::V6(v6) => {
                    let b = u128::from(v6);
                    (b as u64) ^ ((b >> 64) as u64)
                }
            };
            hash ^= bits.wrapping_mul(0x100000001b3);
        }
        Some(hash)
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

            // Cache miss — resolve via async hickory resolver.
            //
            // For permanent DNS failures (NXDOMAIN or NOERROR-with-no-records),
            // wrap the error as `io::Error(ErrorKind::NotFound, ...)` so the
            // downstream `page::is_dns_error` fast path (downcast + kind check)
            // classifies the connect failure as a permanent 525 DNS error
            // without falling back to string scanning. Transient resolver
            // errors (timeouts, I/O, protocol) are passed through untyped so
            // they remain retryable via the normal connect-error path.
            //
            // Same allocation count as before: one Box on the error path.
            let resolver =
                async_resolver().ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(std::io::Error::other("hickory resolver unavailable"))
                })?;
            let lookup = resolver.lookup_ip(&host).await.map_err(
                |e| -> Box<dyn std::error::Error + Send + Sync> {
                    if e.is_no_records_found() {
                        Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, e))
                    } else {
                        Box::new(e)
                    }
                },
            )?;

            let ips: Vec<IpAddr> = lookup.iter().collect();
            if ips.is_empty() {
                // Treat empty-but-successful lookups as permanent DNS
                // failures — there is no record to connect to. Using a
                // NotFound io::Error matches the classification path above.
                let empty: Box<dyn std::error::Error + Send + Sync> =
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "dns resolution returned no addresses",
                    ));
                return Err(empty);
            }

            let sockaddrs: Arc<[SocketAddr]> = ips
                .iter()
                .map(|ip| SocketAddr::new(*ip, 0))
                .collect::<Vec<_>>()
                .into();

            cache.cache.insert(
                host,
                DnsEntry {
                    sockaddrs: sockaddrs.clone(),
                    addrs: ips,
                    expires: Instant::now() + cache.ttl,
                },
            );

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

impl DnsCacheResolver {
    /// Extract the hostname from a proxy URL (e.g. `http://user:pass@host:port`).
    fn parse_proxy_host(proxy_url: &str) -> Option<String> {
        url::Url::parse(proxy_url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
    }

    /// Pre-resolve proxy hostnames so the first request through each proxy
    /// hits a warm DNS cache. Call once at crawl startup.
    pub async fn prefetch_proxy_hosts(&self, proxy_urls: &[String]) {
        let hosts: Vec<&str> = proxy_urls
            .iter()
            .filter_map(|u| {
                url::Url::parse(u)
                    .ok()
                    .and_then(|parsed| parsed.host_str().map(|_| u.as_str()))
            })
            .collect::<Vec<_>>();

        // Collect unique hostnames.
        let mut unique = Vec::with_capacity(hosts.len());
        let mut seen = std::collections::HashSet::with_capacity(proxy_urls.len());
        for url_str in proxy_urls {
            if let Some(host) = Self::parse_proxy_host(url_str) {
                if seen.insert(host.clone()) {
                    unique.push(host);
                }
            }
        }

        if !unique.is_empty() {
            let refs: Vec<&str> = unique.iter().map(|s| s.as_str()).collect();
            self.0.pre_resolve(&refs).await;
        }
    }

    /// Invalidate the DNS entry for a proxy URL so the next connection
    /// re-resolves. Call when a proxy connection fails.
    pub fn invalidate_proxy(&self, proxy_url: &str) {
        if let Some(host) = Self::parse_proxy_host(proxy_url) {
            self.0.invalidate(&host);
        }
    }

    /// Spawn an adaptive background task that keeps proxy-host DNS warm.
    ///
    /// - Starts checking 30 s after spawn.
    /// - Backs off to 4 min when IPs are stable (well under 5-min TTL).
    /// - Resets to 30 s on IP change (proxy rotation / failover).
    /// - ±20 % jitter avoids thundering herd across concurrent crawlers.
    ///
    /// Returns a `JoinHandle` — drop or abort when the crawl ends.
    /// No-op (immediately-ready handle) if `proxy_urls` is empty.
    pub fn spawn_proxy_dns_refresh(&self, proxy_urls: &[String]) -> tokio::task::JoinHandle<()> {
        let mut unique_hosts = Vec::with_capacity(proxy_urls.len());
        let mut seen = std::collections::HashSet::with_capacity(proxy_urls.len());
        for url_str in proxy_urls {
            if let Some(host) = Self::parse_proxy_host(url_str) {
                if seen.insert(host.clone()) {
                    unique_hosts.push(host);
                }
            }
        }

        if unique_hosts.is_empty() {
            return tokio::spawn(async {});
        }

        let cache = self.0.clone();

        tokio::spawn(async move {
            // Seed initial hashes.
            let mut last_hashes: Vec<Option<u64>> = Vec::with_capacity(unique_hosts.len());
            for host in &unique_hosts {
                last_hashes.push(cache.resolve_hash(host).await);
            }

            let mut interval_secs = PROXY_REFRESH_MIN_SECS;
            let mut jitter_counter: u64 = 0;

            loop {
                // Jitter via counter mixing — no syscall.
                jitter_counter = jitter_counter.wrapping_add(0x9e3779b97f4a7c15);
                let factor = 0.8 + (((jitter_counter >> 33) as f64) / (u32::MAX as f64)) * 0.4;
                let sleep_ms = (interval_secs as f64 * factor * 1000.0) as u64;
                tokio::time::sleep(Duration::from_millis(sleep_ms)).await;

                let mut any_changed = false;
                for (i, host) in unique_hosts.iter().enumerate() {
                    let current = cache.resolve_hash(host).await;
                    if current != last_hashes[i] {
                        log::info!("proxy DNS changed for {host}, resetting refresh interval");
                        last_hashes[i] = current;
                        any_changed = true;
                    }
                }

                if any_changed {
                    interval_secs = PROXY_REFRESH_MIN_SECS;
                } else {
                    interval_secs = (interval_secs * 2).min(PROXY_REFRESH_MAX_SECS);
                }
            }
        })
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
        assert!(!cache.is_empty());
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
