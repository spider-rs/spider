/// Parallel crawl backends — race alternative browser engines (LightPanda, Servo)
/// alongside the primary crawl path. Lock-free, panic-free, zero overhead when disabled.
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
#[cfg(any(feature = "lightpanda", feature = "servo"))]
use std::time::Instant;

use crate::configuration::{
    BackendEndpoint, BackendEngine, BackendProtocol, ParallelBackendsConfig, ProxyIgnore,
    RequestProxy,
};
use crate::page::AntiBotTech;
use reqwest::StatusCode;

// ---------------------------------------------------------------------------
// Asset / Binary Content-Type Detection
// ---------------------------------------------------------------------------

/// Returns `true` for `Content-Type` values where HTML quality racing is
/// pointless — binary resources (images, fonts, video, archives, etc.)
/// will be identical across all backends.
pub fn is_binary_content_type(ct: &str) -> bool {
    let ct = ct.split(';').next().unwrap_or(ct).trim();
    ct.starts_with("image/")
        || ct.starts_with("audio/")
        || ct.starts_with("video/")
        || ct.starts_with("font/")
        || ct == "application/pdf"
        || ct == "application/zip"
        || ct == "application/gzip"
        || ct == "application/x-gzip"
        || ct == "application/octet-stream"
        || ct == "application/wasm"
        || ct == "application/x-tar"
        || ct == "application/x-bzip2"
        || ct == "application/x-7z-compressed"
        || ct == "application/x-rar-compressed"
        || ct == "application/vnd.ms-fontobject"
        || ct == "application/x-font-ttf"
        || ct == "application/x-font-woff"
}

/// Returns `true` when the URL extension indicates a binary asset or matches
/// a user-supplied skip extension. Backends should not be spawned for these.
pub fn should_skip_backend_for_url(
    url: &str,
    extra_extensions: &[crate::compact_str::CompactString],
) -> bool {
    // Check the built-in asset list first.
    if crate::page::is_asset_url(url) {
        return true;
    }
    // Check user-supplied extra extensions.
    if !extra_extensions.is_empty() {
        if let Some(pos) = url.rfind('.') {
            let ext = &url[pos + 1..];
            if ext.len() >= 2 {
                let ext_lower = ext.to_ascii_lowercase();
                for skip in extra_extensions {
                    if skip.eq_ignore_ascii_case(&ext_lower) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Custom Validator
// ---------------------------------------------------------------------------

/// The result of a custom quality validation.
#[derive(Default)]
pub struct ValidationResult {
    /// Override the built-in score entirely. When `Some`, the built-in
    /// scorer is bypassed and this value is used directly (0–100).
    pub score_override: Option<u16>,
    /// Additive adjustment applied on top of the built-in score.
    /// Positive values boost, negative values penalise. Applied after
    /// the built-in scorer, before clamping to 0–100.
    pub score_adjust: i16,
    /// When `true`, reject this response outright (treat as score 0).
    pub reject: bool,
}

/// User-supplied quality validator. Called after the built-in scorer for
/// every backend response. Receives the raw HTML bytes, status code, URL,
/// and the backend source name ("primary", "lightpanda", "servo", "custom").
///
/// Must be `Send + Sync` so it can be shared across async tasks.
pub type QualityValidator = std::sync::Arc<
    dyn Fn(
            Option<&[u8]>, // html content
            StatusCode,    // status code
            &str,          // url
            &str,          // backend source name
        ) -> ValidationResult
        + Send
        + Sync,
>;

// ---------------------------------------------------------------------------
// HTML Quality Scorer
// ---------------------------------------------------------------------------

/// Score an HTML response with both the built-in scorer and an optional
/// custom validator. Returns the final clamped score (0–100).
pub fn html_quality_score_validated(
    content: Option<&[u8]>,
    status_code: StatusCode,
    anti_bot: &AntiBotTech,
    url: &str,
    source: &str,
    validator: Option<&QualityValidator>,
) -> u16 {
    let base = html_quality_score(content, status_code, anti_bot);

    if let Some(v) = validator {
        let result = v(content, status_code, url, source);
        if result.reject {
            return 0;
        }
        if let Some(ov) = result.score_override {
            return ov.min(100);
        }
        let adjusted = (base as i16).saturating_add(result.score_adjust);
        return (adjusted.max(0) as u16).min(100);
    }

    base
}

/// Score an HTML response for quality (0–100). Higher is better.
///
/// Used by [`race_backends`] to pick the best response when multiple backends
/// complete within the grace period.
pub fn html_quality_score(
    content: Option<&[u8]>,
    status_code: StatusCode,
    anti_bot: &AntiBotTech,
) -> u16 {
    let mut score: u16 = 0;

    // Status code contribution (max 30).
    if status_code == StatusCode::OK {
        score += 30;
    } else if status_code.is_success() {
        score += 20;
    } else if status_code.is_redirection() {
        score += 5;
    }
    // 4xx / 5xx contribute 0.

    if let Some(body) = content {
        let len = body.len();

        // Content length contribution (max 25).
        if len > 0 {
            score += 5;
        }
        if len > 512 {
            score += 10;
        }
        if len > 4096 {
            score += 10;
        }

        // Has a <body tag (max 15). Fast memchr scan.
        if memchr::memmem::find(body, b"<body").is_some()
            || memchr::memmem::find(body, b"<BODY").is_some()
        {
            score += 15;
        }

        // Not an empty HTML shell (max 10).
        if !crate::utils::is_cacheable_body_empty(body) {
            score += 10;
        }
    }

    // Anti-bot contribution (max 20).
    if *anti_bot == AntiBotTech::None {
        score += 20;
    }

    score.min(100)
}

// ---------------------------------------------------------------------------
// Backend Tracker — lock-free per-backend statistics
// ---------------------------------------------------------------------------

/// Per-backend atomic statistics.
struct BackendStats {
    wins: AtomicU64,
    races: AtomicU64,
    ema_ms: AtomicU64,
    consecutive_errors: AtomicU64,
    disabled: AtomicBool,
}

impl BackendStats {
    fn new() -> Self {
        Self {
            wins: AtomicU64::new(0),
            races: AtomicU64::new(0),
            ema_ms: AtomicU64::new(0),
            consecutive_errors: AtomicU64::new(0),
            disabled: AtomicBool::new(false),
        }
    }
}

impl Clone for BackendStats {
    fn clone(&self) -> Self {
        Self {
            wins: AtomicU64::new(self.wins.load(Ordering::Relaxed)),
            races: AtomicU64::new(self.races.load(Ordering::Relaxed)),
            ema_ms: AtomicU64::new(self.ema_ms.load(Ordering::Relaxed)),
            consecutive_errors: AtomicU64::new(self.consecutive_errors.load(Ordering::Relaxed)),
            disabled: AtomicBool::new(self.disabled.load(Ordering::Relaxed)),
        }
    }
}

/// Tracks per-backend performance across a crawl session.
///
/// Fully lock-free — uses atomics only. Index 0 is the primary backend,
/// 1..N are the alternatives in the order they appear in the config.
pub struct BackendTracker {
    stats: Vec<BackendStats>,
    max_consecutive_errors: u64,
}

impl BackendTracker {
    /// Create a new tracker for `count` backends (primary + alternatives).
    pub fn new(count: usize, max_consecutive_errors: u16) -> Self {
        let mut stats = Vec::with_capacity(count);
        for _ in 0..count {
            stats.push(BackendStats::new());
        }
        Self {
            stats,
            max_consecutive_errors: max_consecutive_errors as u64,
        }
    }

    /// Record a win for backend at `idx`.
    pub fn record_win(&self, idx: usize) {
        if let Some(s) = self.stats.get(idx) {
            s.wins.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record that backend `idx` participated in a race.
    pub fn record_race(&self, idx: usize) {
        if let Some(s) = self.stats.get(idx) {
            s.races.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a fetch duration for backend `idx` (EMA with alpha ~0.2).
    pub fn record_duration(&self, idx: usize, dur: Duration) {
        if let Some(s) = self.stats.get(idx) {
            let ms = dur.as_millis() as u64;
            let count = s.races.load(Ordering::Relaxed);
            if count <= 1 {
                s.ema_ms.store(ms, Ordering::Relaxed);
            } else {
                let old = s.ema_ms.load(Ordering::Relaxed);
                s.ema_ms.store((old * 4 + ms) / 5, Ordering::Relaxed);
            }
        }
    }

    /// Record a retryable error for backend `idx`.
    ///
    /// The first request to each backend acts as a probe: if the backend
    /// has never succeeded (zero prior successes / zero wins), the first
    /// error disables it immediately so a down backend is never retried.
    /// After at least one success, the normal `max_consecutive_errors`
    /// threshold applies.
    pub fn record_error(&self, idx: usize) {
        if let Some(s) = self.stats.get(idx) {
            let prev = s.consecutive_errors.fetch_add(1, Ordering::Relaxed);

            // Probe behaviour: backend has never produced a successful
            // response → treat the very first error as fatal.
            let never_succeeded =
                s.wins.load(Ordering::Relaxed) == 0 && s.races.load(Ordering::Relaxed) <= 1;

            if never_succeeded || prev + 1 >= self.max_consecutive_errors {
                s.disabled.store(true, Ordering::Relaxed);
                if never_succeeded {
                    log::info!(
                        "[parallel_backends] backend {} failed on probe (first request) — auto-disabled",
                        idx
                    );
                }
            }
        }
    }

    /// Record a successful fetch — resets the consecutive error counter.
    pub fn record_success(&self, idx: usize) {
        if let Some(s) = self.stats.get(idx) {
            s.consecutive_errors.store(0, Ordering::Relaxed);
        }
    }

    /// Check whether backend `idx` has been auto-disabled.
    pub fn is_disabled(&self, idx: usize) -> bool {
        self.stats
            .get(idx)
            .is_none_or(|s| s.disabled.load(Ordering::Relaxed))
    }

    /// Get the win count for backend `idx`.
    pub fn wins(&self, idx: usize) -> u64 {
        self.stats
            .get(idx)
            .map_or(0, |s| s.wins.load(Ordering::Relaxed))
    }

    /// Get the race count for backend `idx`.
    pub fn races(&self, idx: usize) -> u64 {
        self.stats
            .get(idx)
            .map_or(0, |s| s.races.load(Ordering::Relaxed))
    }

    /// Get the EMA response time in ms for backend `idx`.
    pub fn ema_ms(&self, idx: usize) -> u64 {
        self.stats
            .get(idx)
            .map_or(0, |s| s.ema_ms.load(Ordering::Relaxed))
    }

    /// Get the consecutive error count for backend `idx`.
    pub fn consecutive_errors(&self, idx: usize) -> u64 {
        self.stats
            .get(idx)
            .map_or(0, |s| s.consecutive_errors.load(Ordering::Relaxed))
    }

    /// Win rate percentage (0–100) for backend `idx`. Returns 0 if no races.
    pub fn win_rate_pct(&self, idx: usize) -> u64 {
        let r = self.races(idx);
        if r == 0 {
            return 0;
        }
        self.wins(idx) * 100 / r
    }

    /// Number of tracked backends.
    pub fn len(&self) -> usize {
        self.stats.len()
    }

    /// Returns true if no backends are tracked.
    pub fn is_empty(&self) -> bool {
        self.stats.is_empty()
    }
}

impl Clone for BackendTracker {
    fn clone(&self) -> Self {
        Self {
            stats: self.stats.clone(),
            max_consecutive_errors: self.max_consecutive_errors,
        }
    }
}

// ---------------------------------------------------------------------------
// Backend Response
// ---------------------------------------------------------------------------

/// The result of a backend page fetch, carrying quality metadata.
pub struct BackendResponse {
    /// The fetched page.
    pub page: crate::page::Page,
    /// Quality score (0–100) computed by [`html_quality_score`].
    pub quality_score: u16,
    /// Backend index: 0 = primary, 1..N = alternatives.
    pub backend_index: usize,
    /// Wall-clock duration of the fetch.
    pub duration: Duration,
}

/// Wrapper returned by backend futures — always carries the backend index
/// so that failures can be tracked for auto-disable.
pub struct BackendResult {
    /// The backend index (0 = primary, 1..N = alternatives).
    pub backend_index: usize,
    /// `Some` on success, `None` on failure.
    pub response: Option<BackendResponse>,
}

/// Return a human-readable backend source name for the given config entry.
pub fn backend_source_name(endpoint: &BackendEndpoint) -> &'static str {
    match endpoint.engine {
        BackendEngine::LightPanda => "lightpanda",
        BackendEngine::Servo => "servo",
        BackendEngine::Custom => "custom",
    }
}

/// Resolve the protocol for a backend endpoint. Falls back to engine defaults.
pub fn resolve_protocol(endpoint: &BackendEndpoint) -> BackendProtocol {
    if let Some(ref p) = endpoint.protocol {
        return p.clone();
    }
    match endpoint.engine {
        BackendEngine::LightPanda => BackendProtocol::Cdp,
        BackendEngine::Servo => BackendProtocol::WebDriver,
        BackendEngine::Custom => {
            // Infer from URL scheme if possible.
            if let Some(ref ep) = endpoint.endpoint {
                if ep.starts_with("ws://") || ep.starts_with("wss://") {
                    return BackendProtocol::Cdp;
                }
            }
            BackendProtocol::WebDriver // default fallback
        }
    }
}

/// Set the `backend_source` field on a page (feature-gated).
#[inline]
pub fn tag_page_source(page: &mut crate::page::Page, source: &str) {
    page.backend_source = Some(crate::compact_str::CompactString::from(source));
}

// ---------------------------------------------------------------------------
// Race Orchestrator
// ---------------------------------------------------------------------------

/// Race the primary crawl against alternative backend futures.
///
/// 1. All futures start immediately.
/// 2. When the first `Some` result arrives:
///    - If `quality_score >= fast_accept_threshold`, return immediately.
///    - Otherwise, start the grace period timer.
/// 3. During the grace period, collect additional results.
/// 4. After the grace period (or when all futures complete), pick the
///    highest-scoring result.
/// 5. Remaining futures are cancelled via drop.
///
/// Returns `None` only when every future returns `None`.
pub async fn race_backends(
    primary: Pin<Box<dyn Future<Output = Option<BackendResponse>> + Send>>,
    alternatives: Vec<Pin<Box<dyn Future<Output = BackendResult> + Send>>>,
    config: &ParallelBackendsConfig,
    tracker: &BackendTracker,
) -> Option<BackendResponse> {
    if !config.enabled || alternatives.is_empty() {
        // No alternatives — just run primary.
        let resp = primary.await;
        if let Some(ref r) = resp {
            tracker.record_race(r.backend_index);
            tracker.record_win(r.backend_index);
            tracker.record_duration(r.backend_index, r.duration);
            tracker.record_success(r.backend_index);
        }
        return resp;
    }

    let total = 1 + alternatives.len();

    // Randomise launch order: sometimes the primary goes first, sometimes
    // a backend does. This prevents predictable timing patterns that could
    // be fingerprinted. The primary gets a random 0–3ms jitter; backends
    // already have their own jitter from build_backend_futures.
    let primary_jitter_us = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        std::time::SystemTime::now().hash(&mut h);
        0u16.hash(&mut h); // primary marker
        h.finish() % 3000 // 0–3ms
    };

    let primary_wrapped: Pin<Box<dyn Future<Output = BackendResult> + Send>> =
        Box::pin(async move {
            if primary_jitter_us > 0 {
                tokio::time::sleep(Duration::from_micros(primary_jitter_us)).await;
            }
            let response = primary.await;
            BackendResult {
                backend_index: 0,
                response,
            }
        });

    let mut futs = tokio::task::JoinSet::new();
    futs.spawn(primary_wrapped);
    for alt in alternatives {
        futs.spawn(alt);
    }

    let grace = Duration::from_millis(config.grace_period_ms);
    let threshold = config.fast_accept_threshold;

    let mut best: Option<BackendResponse> = None;
    let mut completed = 0usize;
    let mut grace_deadline: Option<tokio::time::Instant> = None;

    loop {
        if completed >= total {
            break;
        }

        let result = if let Some(deadline) = grace_deadline {
            tokio::select! {
                biased;
                res = futs.join_next() => res,
                _ = tokio::time::sleep_until(deadline) => break,
            }
        } else {
            futs.join_next().await
        };

        match result {
            Some(Ok(br)) => {
                completed += 1;
                let idx = br.backend_index;

                match br.response {
                    Some(resp) => {
                        tracker.record_race(idx);
                        tracker.record_duration(idx, resp.duration);
                        tracker.record_success(idx);

                        let score = resp.quality_score;

                        // Fast-accept: high-quality first response skips grace.
                        if best.is_none() && score >= threshold {
                            tracker.record_win(idx);
                            return Some(resp);
                        }

                        let dominated = match &best {
                            Some(b) => score > b.quality_score,
                            None => true,
                        };
                        if dominated {
                            best = Some(resp);
                        }

                        if grace_deadline.is_none() {
                            grace_deadline = Some(tokio::time::Instant::now() + grace);
                        }
                    }
                    None => {
                        // Backend failed — track error for auto-disable.
                        tracker.record_race(idx);
                        tracker.record_error(idx);
                    }
                }
            }
            Some(Err(_join_err)) => {
                completed += 1;
            }
            None => {
                break;
            }
        }
    }

    if let Some(ref b) = best {
        tracker.record_win(b.backend_index);
    }

    best
}

// ---------------------------------------------------------------------------
// Proxy Rotator — lock-free round-robin
// ---------------------------------------------------------------------------

/// Round-robin proxy address selector for parallel backends.
///
/// Pre-filters proxy lists for CDP (LightPanda) and WebDriver (Servo)
/// based on [`ProxyIgnore`]. Lock-free via [`AtomicUsize`].
pub struct ProxyRotator {
    /// Proxies for CDP backends (filtered: `ProxyIgnore != Chrome`).
    cdp_addrs: Vec<String>,
    /// Proxies for WebDriver backends (filtered: `ProxyIgnore != Http`).
    wd_addrs: Vec<String>,
    cdp_index: AtomicUsize,
    wd_index: AtomicUsize,
}

impl ProxyRotator {
    /// Build from the crawler's proxy list.
    pub fn new(proxies: &Option<Vec<RequestProxy>>) -> Self {
        let (mut cdp, mut wd) = (Vec::new(), Vec::new());

        if let Some(proxies) = proxies {
            for p in proxies {
                if p.ignore != ProxyIgnore::Chrome {
                    cdp.push(p.addr.clone());
                }
                if p.ignore != ProxyIgnore::Http {
                    wd.push(p.addr.clone());
                }
            }
        }

        Self {
            cdp_addrs: cdp,
            wd_addrs: wd,
            cdp_index: AtomicUsize::new(0),
            wd_index: AtomicUsize::new(0),
        }
    }

    /// Get the next CDP proxy address (round-robin). Returns `None` if empty.
    pub fn next_cdp(&self) -> Option<&str> {
        let len = self.cdp_addrs.len();
        if len == 0 {
            return None;
        }
        let idx = self.cdp_index.fetch_add(1, Ordering::Relaxed) % len;
        self.cdp_addrs.get(idx).map(|s| s.as_str())
    }

    /// Get the next WebDriver proxy address (round-robin). Returns `None` if empty.
    pub fn next_webdriver(&self) -> Option<&str> {
        let len = self.wd_addrs.len();
        if len == 0 {
            return None;
        }
        let idx = self.wd_index.fetch_add(1, Ordering::Relaxed) % len;
        self.wd_addrs.get(idx).map(|s| s.as_str())
    }

    /// Number of CDP proxies available.
    pub fn cdp_count(&self) -> usize {
        self.cdp_addrs.len()
    }

    /// Number of WebDriver proxies available.
    pub fn webdriver_count(&self) -> usize {
        self.wd_addrs.len()
    }
}

impl Clone for ProxyRotator {
    fn clone(&self) -> Self {
        Self {
            cdp_addrs: self.cdp_addrs.clone(),
            wd_addrs: self.wd_addrs.clone(),
            cdp_index: AtomicUsize::new(self.cdp_index.load(Ordering::Relaxed)),
            wd_index: AtomicUsize::new(self.wd_index.load(Ordering::Relaxed)),
        }
    }
}

// ---------------------------------------------------------------------------
// Backend Fetch Functions
// ---------------------------------------------------------------------------

/// Fetch a page via a remote LightPanda CDP endpoint.
///
/// Fresh CDP connection per fetch with the **same handler config** as the
/// primary Chrome path — network interception, resource blocking, viewport,
/// timeouts all pass through transparently via `connect_with_config()`.
/// Fully lock-free: no mutexes, no semaphores, no locks.
#[cfg(feature = "lightpanda")]
pub async fn fetch_lightpanda_cdp(
    url: &str,
    endpoint: &str,
    config: &std::sync::Arc<crate::configuration::Configuration>,
    backend_index: usize,
    connect_timeout: Duration,
    proxy: Option<String>,
) -> Option<BackendResponse> {
    let start = Instant::now();
    let timeout = config.request_timeout.unwrap_or(Duration::from_secs(15));

    // Build the same handler config as the primary Chrome crawl path.
    // This gives LightPanda identical network interception: block_visuals,
    // block_javascript, block_stylesheets, block_ads, block_analytics,
    // whitelist/blacklist patterns, extra headers, viewport, etc.
    let handler_config = crate::features::chrome::create_handler_config(config);

    // Connect with a short timeout so down backends fail fast.
    let connect_result = tokio::time::timeout(
        connect_timeout,
        chromiumoxide::Browser::connect_with_config(endpoint, handler_config),
    )
    .await;

    let (mut browser, handler_handle) = match connect_result {
        Ok(Ok((browser, mut handler))) => {
            let h = tokio::spawn(async move {
                use crate::tokio_stream::StreamExt;
                while let Some(_) = handler.next().await {}
            });
            (browser, h)
        }
        Ok(Err(e)) => {
            log::warn!("LightPanda CDP connect failed ({}): {:?}", endpoint, e);
            return None;
        }
        Err(_) => {
            log::warn!("LightPanda CDP connect timed out ({})", endpoint);
            return None;
        }
    };

    // If a proxy is configured, create an isolated browser context with
    // proxy_server so this backend's requests route through it.
    if let Some(ref proxy_addr) = proxy {
        let mut ctx_params =
            chromiumoxide::cdp::browser_protocol::target::CreateBrowserContextParams::default();
        ctx_params.dispose_on_detach = Some(true);
        ctx_params.proxy_server = Some(proxy_addr.clone());
        if let Ok(ctx) = browser.create_browser_context(ctx_params).await {
            let _ = browser.send_new_context(ctx).await;
        } else {
            log::warn!(
                "LightPanda proxy browser context failed for {}, continuing without proxy",
                proxy_addr
            );
        }
    }

    // Get the default page.
    let page = match browser.pages().await {
        Ok(mut p) if !p.is_empty() => p.swap_remove(0),
        _ => match browser.new_page(url).await {
            Ok(p) => p,
            Err(e) => {
                log::warn!("LightPanda page failed: {:?}", e);
                handler_handle.abort();
                return None;
            }
        },
    };

    // Apply the same page-level config as the primary Chrome path.
    crate::features::chrome::setup_chrome_events(&page, config).await;

    // Auth challenge interception if enabled.
    let _intercept_handle = crate::features::chrome::setup_chrome_interception_base(
        &page,
        config.chrome_intercept.enabled,
        &config.auth_challenge_response,
        config.chrome_intercept.block_visuals,
        "",
    )
    .await;

    // Navigate.
    match tokio::time::timeout(timeout, page.goto(url)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            log::warn!("LightPanda navigate failed for {}: {:?}", url, e);
            handler_handle.abort();
            return None;
        }
        Err(_) => {
            log::warn!("LightPanda navigate timed out for {}", url);
            handler_handle.abort();
            return None;
        }
    }

    // Wait for load event if configured.
    #[cfg(feature = "chrome")]
    if let Some(ref wf) = config.wait_for {
        if let Some(ref delay) = wf.delay {
            if let Some(ms) = delay.timeout {
                tokio::time::sleep(ms).await;
            }
        }
    }

    // Get the outer HTML.
    let html_result = tokio::time::timeout(Duration::from_secs(10), page.outer_html_bytes()).await;

    // Clean up.
    handler_handle.abort();

    let html_bytes: Vec<u8> = match html_result {
        Ok(Ok(b)) => b.to_vec(),
        Ok(Err(e)) => {
            log::warn!("LightPanda outer_html_bytes() failed for {}: {:?}", url, e);
            return None;
        }
        Err(_) => {
            log::warn!("LightPanda outer_html_bytes() timed out for {}", url);
            return None;
        }
    };

    let dur = start.elapsed();
    let status = StatusCode::OK;

    let score = html_quality_score(Some(&html_bytes), status, &AntiBotTech::None);
    let res = crate::utils::PageResponse {
        content: Some(html_bytes),
        status_code: status,
        ..Default::default()
    };
    let mut page = crate::page::build(url, res);
    tag_page_source(&mut page, "lightpanda");

    Some(BackendResponse {
        page,
        quality_score: score,
        backend_index,
        duration: dur,
    })
}

/// Fetch a page via a remote Servo WebDriver endpoint.
///
/// Reuses the existing `thirtyfour` / `webdriver.rs` infrastructure.
#[cfg(feature = "servo")]
pub async fn fetch_servo_webdriver(
    url: &str,
    endpoint: &str,
    config: &std::sync::Arc<crate::configuration::Configuration>,
    backend_index: usize,
    connect_timeout: Duration,
    proxy: Option<String>,
) -> Option<BackendResponse> {
    use crate::features::webdriver_common::{WebDriverBrowser, WebDriverConfig};

    let start = Instant::now();
    let timeout = config.request_timeout.unwrap_or(Duration::from_secs(15));

    // Build a WebDriverConfig pointing at the Servo endpoint.
    let wd_config = WebDriverConfig {
        server_url: endpoint.to_string(),
        browser: WebDriverBrowser::Chrome, // Servo's WebDriver is browser-agnostic
        headless: true,
        timeout: Some(connect_timeout),
        proxy, // Per-backend proxy or ProxyRotator fallback
        user_agent: config.user_agent.as_ref().map(|ua| ua.to_string()),
        viewport_width: config.viewport.as_ref().map(|v| v.width),
        viewport_height: config.viewport.as_ref().map(|v| v.height),
        accept_insecure_certs: config.accept_invalid_certs,
        ..Default::default()
    };

    // Launch session with a short connect timeout so down backends fail fast.
    let controller_opt = tokio::time::timeout(
        connect_timeout,
        crate::features::webdriver::launch_driver_base(&wd_config, config),
    )
    .await;

    let mut controller = match controller_opt {
        Ok(Some(c)) => c,
        Ok(None) => {
            log::warn!("Servo WebDriver connect failed ({})", endpoint);
            return None;
        }
        Err(_) => {
            log::warn!("Servo WebDriver connect timed out ({})", endpoint);
            return None;
        }
    };

    let driver = controller.driver().clone();

    // Navigate with timeout.
    match tokio::time::timeout(timeout, driver.goto(url)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            log::warn!("Servo WebDriver navigate failed for {}: {:?}", url, e);
            controller.dispose();
            return None;
        }
        Err(_) => {
            log::warn!("Servo WebDriver navigate timed out for {}", url);
            controller.dispose();
            return None;
        }
    }

    // Get page source with timeout.
    let source = match tokio::time::timeout(Duration::from_secs(10), driver.source()).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            log::warn!("Servo WebDriver source failed for {}: {:?}", url, e);
            controller.dispose();
            return None;
        }
        Err(_) => {
            log::warn!("Servo WebDriver source timed out for {}", url);
            controller.dispose();
            return None;
        }
    };

    controller.dispose();

    let dur = start.elapsed();
    let html_bytes = source.into_bytes();
    let status = StatusCode::OK;

    let score = html_quality_score(Some(&html_bytes), status, &AntiBotTech::None);
    let res = crate::utils::PageResponse {
        content: Some(html_bytes),
        status_code: status,
        ..Default::default()
    };
    let mut page = crate::page::build(url, res);
    tag_page_source(&mut page, "servo");

    Some(BackendResponse {
        page,
        quality_score: score,
        backend_index,
        duration: dur,
    })
}

// ---------------------------------------------------------------------------
// Builder Helper
// ---------------------------------------------------------------------------

/// Build alternative backend futures for a given URL from config.
///
/// Skips backends that have been auto-disabled by the tracker.
/// Build alternative backend futures for a given URL from config.
///
/// Skips backends that have been auto-disabled by the tracker.
/// Accepts `Arc<Configuration>` to avoid per-URL deep clones.
#[allow(unused_variables)]
pub fn build_backend_futures(
    url: &str,
    config: &ParallelBackendsConfig,
    crawl_config: &std::sync::Arc<crate::configuration::Configuration>,
    tracker: &BackendTracker,
    proxy_rotator: &Option<std::sync::Arc<ProxyRotator>>,
    semaphore: &Option<std::sync::Arc<tokio::sync::Semaphore>>,
) -> Vec<Pin<Box<dyn Future<Output = BackendResult> + Send>>> {
    // Fast-path: skip backends for binary/asset URLs. There is no HTML
    // quality variance for images, fonts, PDFs, etc.
    if should_skip_backend_for_url(url, &config.skip_extensions) {
        log::debug!(
            "[parallel_backends] skipping backends for asset URL: {}",
            url
        );
        return Vec::new();
    }

    #[allow(unused_mut)]
    let mut futs: Vec<Pin<Box<dyn Future<Output = BackendResult> + Send>>> = Vec::new();

    for (i, backend) in config.backends.iter().enumerate() {
        let backend_index = i + 1; // 0 is primary

        if tracker.is_disabled(backend_index) {
            continue;
        }

        // Resolve the endpoint — remote uses `endpoint`, local uses `binary_path`
        // (local mode spawns the engine and connects to localhost).
        #[allow(unused_variables)]
        let resolved_endpoint = if let Some(ref ep) = backend.endpoint {
            ep.clone()
        } else if backend.binary_path.is_some() {
            log::debug!(
                "{:?} local mode not yet implemented, skipping",
                backend.engine
            );
            continue;
        } else {
            log::debug!(
                "{:?} backend has no endpoint or binary_path, skipping",
                backend.engine
            );
            continue;
        };

        let proto = resolve_protocol(backend);
        let _source_name = backend_source_name(backend);

        // Resolve proxy: per-backend proxy takes priority, then ProxyRotator fallback.
        #[allow(unused_variables)]
        let resolved_proxy: Option<String> = if backend.proxy.is_some() {
            backend.proxy.clone()
        } else if let Some(ref rotator) = proxy_rotator {
            match proto {
                BackendProtocol::Cdp => rotator.next_cdp().map(|s| s.to_string()),
                BackendProtocol::WebDriver => rotator.next_webdriver().map(|s| s.to_string()),
            }
        } else {
            None
        };

        // Random sub-ms to 5ms jitter before each backend launch to prevent
        // fingerprint correlation from simultaneous requests. Uses a cheap
        // hash-based PRNG (no allocations, no syscalls).
        let jitter_us = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            url.hash(&mut hasher);
            backend_index.hash(&mut hasher);
            std::time::SystemTime::now().hash(&mut hasher);
            hasher.finish() % 5000 + 100 // 100µs – 5100µs (~0.1ms – 5ms)
        };

        let connect_timeout = Duration::from_millis(config.connect_timeout_ms);

        // Clone semaphore Arc for the spawned future (cheap Arc clone).
        let sem = semaphore.clone();

        match proto {
            #[cfg(feature = "lightpanda")]
            BackendProtocol::Cdp => {
                let url = url.to_string();
                let cfg = crawl_config.clone(); // Arc clone — cheap
                let proxy = resolved_proxy.clone();
                futs.push(Box::pin(async move {
                    // Acquire semaphore permit before doing any real work.
                    let _permit = if let Some(ref s) = sem {
                        match s.acquire().await {
                            Ok(p) => Some(p),
                            Err(_) => {
                                return BackendResult {
                                    backend_index,
                                    response: None,
                                }
                            }
                        }
                    } else {
                        None
                    };
                    tokio::time::sleep(Duration::from_micros(jitter_us)).await;
                    let response = fetch_lightpanda_cdp(
                        &url,
                        &resolved_endpoint,
                        &cfg,
                        backend_index,
                        connect_timeout,
                        proxy,
                    )
                    .await;
                    BackendResult {
                        backend_index,
                        response,
                    }
                }));
            }
            #[cfg(feature = "servo")]
            BackendProtocol::WebDriver => {
                let url = url.to_string();
                let cfg = crawl_config.clone(); // Arc clone — cheap
                let proxy = resolved_proxy.clone();
                futs.push(Box::pin(async move {
                    // Acquire semaphore permit before doing any real work.
                    let _permit = if let Some(ref s) = sem {
                        match s.acquire().await {
                            Ok(p) => Some(p),
                            Err(_) => {
                                return BackendResult {
                                    backend_index,
                                    response: None,
                                }
                            }
                        }
                    } else {
                        None
                    };
                    tokio::time::sleep(Duration::from_micros(jitter_us)).await;
                    let response = fetch_servo_webdriver(
                        &url,
                        &resolved_endpoint,
                        &cfg,
                        backend_index,
                        connect_timeout,
                        proxy,
                    )
                    .await;
                    BackendResult {
                        backend_index,
                        response,
                    }
                }));
            }
            // When the specific feature is not enabled, skip silently.
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }

    futs
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // ---- Quality Scorer ----

    fn make_html(body_content: &str) -> Vec<u8> {
        format!(
            "<html><head><title>T</title></head><body>{}</body></html>",
            body_content
        )
        .into_bytes()
    }

    #[test]
    fn test_quality_score_perfect_response() {
        let body = make_html(&"x".repeat(5000));
        let score = html_quality_score(Some(&body), StatusCode::OK, &AntiBotTech::None);
        // 30 (200) + 5 (>0) + 10 (>512) + 10 (>4096) + 15 (<body>) + 10 (not empty) + 20 (no bot) = 100
        assert_eq!(score, 100);
    }

    #[test]
    fn test_quality_score_empty_body() {
        let score = html_quality_score(Some(&[]), StatusCode::OK, &AntiBotTech::None);
        // 30 (200) + 0 (empty) + 0 + 0 + 0 + 0 (is_cacheable_body_empty → true for empty) + 20 = 50
        assert_eq!(score, 50);
    }

    #[test]
    fn test_quality_score_none_content() {
        let score = html_quality_score(None, StatusCode::OK, &AntiBotTech::None);
        // 30 (200) + 20 (no bot) = 50
        assert_eq!(score, 50);
    }

    #[test]
    fn test_quality_score_empty_html_shell() {
        let body = b"<html><head></head><body></body></html>";
        let score = html_quality_score(Some(body), StatusCode::OK, &AntiBotTech::None);
        // 30 + 5 (>0) + 0 (38 bytes, <512) + 0 + 15 (<body) + 0 (empty shell) + 20 = 70
        assert_eq!(score, 70);
    }

    #[test]
    fn test_quality_score_antibot_cloudflare() {
        let body = make_html("blocked");
        let score =
            html_quality_score(Some(&body), StatusCode::FORBIDDEN, &AntiBotTech::Cloudflare);
        // 0 (403) + 5 + 0 + 0 + 15 + 10 + 0 (bot!) = 30
        assert_eq!(score, 30);
    }

    #[test]
    fn test_quality_score_server_error() {
        let body = make_html("error");
        let score = html_quality_score(
            Some(&body),
            StatusCode::INTERNAL_SERVER_ERROR,
            &AntiBotTech::None,
        );
        // 0 (500) + 5 + 0 + 0 + 15 + 10 + 20 = 50
        assert_eq!(score, 50);
    }

    #[test]
    fn test_quality_score_redirect() {
        let score = html_quality_score(None, StatusCode::MOVED_PERMANENTLY, &AntiBotTech::None);
        // 5 (301) + 20 = 25
        assert_eq!(score, 25);
    }

    #[test]
    fn test_quality_score_small_body_with_body_tag() {
        let body = b"<html><body>hi</body></html>";
        let score = html_quality_score(Some(body), StatusCode::OK, &AntiBotTech::None);
        // 30 + 5 (>0) + 0 (<512) + 0 + 15 (<body) + 10 (not empty) + 20 = 80
        assert_eq!(score, 80);
    }

    #[test]
    fn test_quality_score_large_body_no_body_tag() {
        let body = "x".repeat(5000);
        let score = html_quality_score(Some(body.as_bytes()), StatusCode::OK, &AntiBotTech::None);
        // 30 + 5 + 10 + 10 + 0 (no <body) + 10 (not empty) + 20 = 85
        assert_eq!(score, 85);
    }

    // ---- Backend Tracker ----

    #[test]
    fn test_tracker_new_defaults() {
        let t = BackendTracker::new(3, 10);
        assert_eq!(t.len(), 3);
        assert!(!t.is_empty());
        for i in 0..3 {
            assert_eq!(t.wins(i), 0);
            assert_eq!(t.races(i), 0);
            assert_eq!(t.ema_ms(i), 0);
            assert_eq!(t.consecutive_errors(i), 0);
            assert!(!t.is_disabled(i));
        }
        // Out-of-bounds returns safe defaults.
        assert!(t.is_disabled(99));
        assert_eq!(t.wins(99), 0);
    }

    #[test]
    fn test_tracker_record_win() {
        let t = BackendTracker::new(2, 10);
        t.record_win(0);
        t.record_win(0);
        t.record_win(1);
        assert_eq!(t.wins(0), 2);
        assert_eq!(t.wins(1), 1);
    }

    #[test]
    fn test_tracker_ema_duration() {
        let t = BackendTracker::new(1, 10);
        t.record_race(0);
        t.record_duration(0, Duration::from_millis(100));
        assert_eq!(t.ema_ms(0), 100);

        t.record_race(0);
        t.record_duration(0, Duration::from_millis(200));
        // EMA = (100 * 4 + 200) / 5 = 120
        assert_eq!(t.ema_ms(0), 120);

        t.record_race(0);
        t.record_duration(0, Duration::from_millis(100));
        // EMA = (120 * 4 + 100) / 5 = 116
        assert_eq!(t.ema_ms(0), 116);
    }

    #[test]
    fn test_tracker_probe_first_error_disables() {
        // A backend that has never succeeded is disabled on first error
        // (probe behaviour).
        let t = BackendTracker::new(1, 10);
        assert!(!t.is_disabled(0));
        t.record_race(0);
        t.record_error(0); // first ever error, no prior wins → probe disable
        assert!(t.is_disabled(0));
    }

    #[test]
    fn test_tracker_consecutive_errors_disables() {
        // After at least one success, max_consecutive_errors threshold applies.
        let t = BackendTracker::new(1, 3);
        // Simulate a successful first request so probe mode doesn't kick in.
        t.record_race(0);
        t.record_win(0);
        t.record_success(0);
        assert!(!t.is_disabled(0));
        t.record_race(0);
        t.record_error(0);
        t.record_race(0);
        t.record_error(0);
        assert!(!t.is_disabled(0));
        t.record_race(0);
        t.record_error(0); // third consecutive error
        assert!(t.is_disabled(0));
    }

    #[test]
    fn test_tracker_success_resets_errors() {
        let t = BackendTracker::new(1, 5);
        // Give it a prior win so probe mode doesn't trigger.
        t.record_race(0);
        t.record_win(0);
        t.record_success(0);
        t.record_race(0);
        t.record_error(0);
        t.record_race(0);
        t.record_error(0);
        assert_eq!(t.consecutive_errors(0), 2);
        t.record_success(0);
        assert_eq!(t.consecutive_errors(0), 0);
    }

    #[test]
    fn test_tracker_clone_independence() {
        let t = BackendTracker::new(1, 10);
        t.record_win(0);
        let t2 = t.clone();
        t.record_win(0);
        assert_eq!(t.wins(0), 2);
        assert_eq!(t2.wins(0), 1);
    }

    #[test]
    fn test_tracker_win_rate() {
        let t = BackendTracker::new(1, 10);
        assert_eq!(t.win_rate_pct(0), 0); // 0 races
        t.record_race(0);
        t.record_race(0);
        t.record_race(0);
        t.record_race(0);
        t.record_win(0);
        t.record_win(0);
        t.record_win(0);
        assert_eq!(t.win_rate_pct(0), 75);
    }

    // ---- Race Orchestrator ----

    /// Mock a successful primary response (returns Option<BackendResponse>).
    fn mock_primary(
        score: u16,
        delay_ms: u64,
    ) -> Pin<Box<dyn Future<Output = Option<BackendResponse>> + Send>> {
        Box::pin(async move {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            Some(BackendResponse {
                page: crate::page::Page::default(),
                quality_score: score,
                backend_index: 0,
                duration: Duration::from_millis(delay_ms),
            })
        })
    }

    /// Mock an alternative backend response (returns BackendResult).
    fn mock_alt(
        idx: usize,
        score: u16,
        delay_ms: u64,
    ) -> Pin<Box<dyn Future<Output = BackendResult> + Send>> {
        Box::pin(async move {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            BackendResult {
                backend_index: idx,
                response: Some(BackendResponse {
                    page: crate::page::Page::default(),
                    quality_score: score,
                    backend_index: idx,
                    duration: Duration::from_millis(delay_ms),
                }),
            }
        })
    }

    /// Mock a failing alternative backend (returns BackendResult with None).
    fn mock_alt_none(
        idx: usize,
        delay_ms: u64,
    ) -> Pin<Box<dyn Future<Output = BackendResult> + Send>> {
        Box::pin(async move {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            BackendResult {
                backend_index: idx,
                response: None,
            }
        })
    }

    /// Mock a failing primary (returns None).
    fn mock_primary_none(
        delay_ms: u64,
    ) -> Pin<Box<dyn Future<Output = Option<BackendResponse>> + Send>> {
        Box::pin(async move {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            None
        })
    }

    fn test_config(grace_ms: u64, threshold: u16) -> ParallelBackendsConfig {
        ParallelBackendsConfig {
            backends: vec![],
            grace_period_ms: grace_ms,
            enabled: true,
            fast_accept_threshold: threshold,
            max_consecutive_errors: 10,
            connect_timeout_ms: 5000,
            skip_binary_content_types: true,
            max_concurrent_sessions: 0,
            skip_extensions: Vec::new(),
        }
    }

    #[tokio::test]
    async fn test_race_primary_fast_accept() {
        let tracker = BackendTracker::new(3, 10);
        let cfg = test_config(500, 80);
        let primary = mock_primary(95, 10);
        let alts = vec![mock_alt(1, 100, 1000), mock_alt(2, 100, 1000)];

        let result = race_backends(primary, alts, &cfg, &tracker).await;
        let r = result.unwrap();
        assert_eq!(r.backend_index, 0); // primary won via fast-accept
        assert_eq!(r.quality_score, 95);
        assert_eq!(tracker.wins(0), 1);
    }

    #[tokio::test]
    async fn test_race_alternative_wins_during_grace() {
        let tracker = BackendTracker::new(3, 10);
        let cfg = test_config(500, 80); // 500ms grace, threshold 80
        let primary = mock_primary(50, 10); // arrives first, low quality
        let alts = vec![
            mock_alt(1, 90, 100), // arrives during grace, high quality
            mock_alt(2, 30, 1000),
        ];

        let result = race_backends(primary, alts, &cfg, &tracker).await;
        let r = result.unwrap();
        assert_eq!(r.backend_index, 1); // alt won with higher score
        assert_eq!(r.quality_score, 90);
    }

    #[tokio::test]
    async fn test_race_primary_wins_after_grace() {
        let tracker = BackendTracker::new(2, 10);
        let cfg = test_config(50, 80); // 50ms grace
        let primary = mock_primary(60, 10); // below threshold
        let alts = vec![
            mock_alt(1, 40, 5000), // too slow, won't arrive during grace
        ];

        let result = race_backends(primary, alts, &cfg, &tracker).await;
        let r = result.unwrap();
        assert_eq!(r.backend_index, 0); // primary best during grace
        assert_eq!(r.quality_score, 60);
    }

    #[tokio::test]
    async fn test_race_all_none() {
        let tracker = BackendTracker::new(2, 10);
        let cfg = test_config(50, 80);
        let primary = mock_primary_none(10);
        let alts = vec![mock_alt_none(1, 10)];

        let result = race_backends(primary, alts, &cfg, &tracker).await;
        assert!(result.is_none());
        // Failed alt should have error recorded.
        assert_eq!(tracker.consecutive_errors(1), 1);
    }

    #[tokio::test]
    async fn test_race_primary_none_alt_some() {
        let tracker = BackendTracker::new(2, 10);
        let cfg = test_config(200, 80);
        let primary = mock_primary_none(10);
        let alts = vec![mock_alt(1, 85, 50)];

        let result = race_backends(primary, alts, &cfg, &tracker).await;
        let r = result.unwrap();
        assert_eq!(r.backend_index, 1);
    }

    #[tokio::test]
    async fn test_race_disabled_noop() {
        let tracker = BackendTracker::new(2, 10);
        let mut cfg = test_config(50, 80);
        cfg.enabled = false;
        let primary = mock_primary(70, 10);
        let alts = vec![mock_alt(1, 100, 10)];

        let result = race_backends(primary, alts, &cfg, &tracker).await;
        let r = result.unwrap();
        assert_eq!(r.backend_index, 0); // disabled → primary only
    }

    #[tokio::test]
    async fn test_race_single_alternative() {
        let tracker = BackendTracker::new(2, 10);
        let cfg = test_config(200, 80);
        let primary = mock_primary(50, 100);
        let alts = vec![mock_alt(1, 90, 20)]; // alt is faster AND better

        let result = race_backends(primary, alts, &cfg, &tracker).await;
        let r = result.unwrap();
        // Alt arrives first with score 90 >= threshold 80 → fast accept
        assert_eq!(r.backend_index, 1);
        assert_eq!(r.quality_score, 90);
    }

    #[tokio::test]
    async fn test_race_three_alternatives_best_during_grace() {
        let tracker = BackendTracker::new(4, 10);
        let cfg = test_config(300, 95); // high threshold

        let primary = mock_primary(40, 10); // first, low quality
        let alts = vec![
            mock_alt(1, 60, 50),  // medium
            mock_alt(2, 85, 100), // best
            mock_alt(3, 70, 200), // arrives within grace but lower
        ];

        let result = race_backends(primary, alts, &cfg, &tracker).await;
        let r = result.unwrap();
        assert_eq!(r.backend_index, 2);
        assert_eq!(r.quality_score, 85);
    }

    #[tokio::test]
    async fn test_race_grace_period_zero() {
        let tracker = BackendTracker::new(2, 10);
        let cfg = test_config(0, 101); // threshold impossibly high, grace = 0

        let primary = mock_primary(50, 10); // arrives first
        let alts = vec![mock_alt(1, 99, 50)]; // better but slower

        let result = race_backends(primary, alts, &cfg, &tracker).await;
        let r = result.unwrap();
        assert_eq!(r.backend_index, 0);
    }

    #[tokio::test]
    async fn test_race_cancellation_verified() {
        let finished = Arc::new(AtomicBool::new(false));
        let f = finished.clone();

        let tracker = BackendTracker::new(2, 10);
        let cfg = test_config(50, 80);

        let primary = mock_primary(95, 10); // fast-accept

        let slow_alt: Pin<Box<dyn Future<Output = BackendResult> + Send>> = Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            f.store(true, Ordering::SeqCst);
            BackendResult {
                backend_index: 1,
                response: None,
            }
        });

        let _result = race_backends(primary, vec![slow_alt], &cfg, &tracker).await;

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!finished.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_race_failed_alt_records_error() {
        let tracker = BackendTracker::new(3, 5);
        let cfg = test_config(200, 80);
        let primary = mock_primary(50, 10);
        let alts = vec![
            mock_alt_none(1, 20), // fails
            mock_alt_none(2, 30), // fails
        ];

        let result = race_backends(primary, alts, &cfg, &tracker).await;
        let r = result.unwrap();
        assert_eq!(r.backend_index, 0); // primary wins since alts failed
        assert_eq!(tracker.consecutive_errors(1), 1);
        assert_eq!(tracker.consecutive_errors(2), 1);
    }

    #[tokio::test]
    async fn test_race_auto_disable_after_errors() {
        let tracker = BackendTracker::new(2, 2); // disable after 2 errors
        let cfg = test_config(100, 80);

        // Run two races where alt fails
        for _ in 0..2 {
            let primary = mock_primary(50, 5);
            let alts = vec![mock_alt_none(1, 10)];
            let _ = race_backends(primary, alts, &cfg, &tracker).await;
        }

        // Backend 1 should now be auto-disabled.
        assert!(tracker.is_disabled(1));
        assert_eq!(tracker.consecutive_errors(1), 2);
    }

    // ---- Proxy Rotator ----

    #[test]
    fn test_proxy_rotator_round_robin_cdp() {
        let proxies = vec![
            RequestProxy {
                addr: "http://p1".into(),
                ignore: ProxyIgnore::No,
            },
            RequestProxy {
                addr: "http://p2".into(),
                ignore: ProxyIgnore::No,
            },
        ];
        let r = ProxyRotator::new(&Some(proxies));
        assert_eq!(r.cdp_count(), 2);
        assert_eq!(r.next_cdp(), Some("http://p1"));
        assert_eq!(r.next_cdp(), Some("http://p2"));
        assert_eq!(r.next_cdp(), Some("http://p1")); // wraps
    }

    #[test]
    fn test_proxy_rotator_round_robin_wd() {
        let proxies = vec![
            RequestProxy {
                addr: "http://p1".into(),
                ignore: ProxyIgnore::No,
            },
            RequestProxy {
                addr: "http://p2".into(),
                ignore: ProxyIgnore::No,
            },
        ];
        let r = ProxyRotator::new(&Some(proxies));
        assert_eq!(r.webdriver_count(), 2);
        assert_eq!(r.next_webdriver(), Some("http://p1"));
        assert_eq!(r.next_webdriver(), Some("http://p2"));
    }

    #[test]
    fn test_proxy_rotator_filters_ignore() {
        let proxies = vec![
            RequestProxy {
                addr: "http://cdp-only".into(),
                ignore: ProxyIgnore::Http, // only for CDP, ignored for WebDriver
            },
            RequestProxy {
                addr: "http://wd-only".into(),
                ignore: ProxyIgnore::Chrome, // only for WebDriver, ignored for CDP
            },
            RequestProxy {
                addr: "http://both".into(),
                ignore: ProxyIgnore::No,
            },
        ];
        let r = ProxyRotator::new(&Some(proxies));
        // CDP: "cdp-only" (Http → not Chrome → included) + "both"
        assert_eq!(r.cdp_count(), 2);
        // WebDriver: "wd-only" (Chrome → not Http → included) + "both"
        assert_eq!(r.webdriver_count(), 2);
    }

    #[test]
    fn test_proxy_rotator_empty_proxies() {
        let r = ProxyRotator::new(&None);
        assert_eq!(r.cdp_count(), 0);
        assert_eq!(r.webdriver_count(), 0);
        assert_eq!(r.next_cdp(), None);
        assert_eq!(r.next_webdriver(), None);
    }

    #[test]
    fn test_proxy_rotator_concurrent_access() {
        let proxies = vec![
            RequestProxy {
                addr: "http://p1".into(),
                ignore: ProxyIgnore::No,
            },
            RequestProxy {
                addr: "http://p2".into(),
                ignore: ProxyIgnore::No,
            },
            RequestProxy {
                addr: "http://p3".into(),
                ignore: ProxyIgnore::No,
            },
        ];
        let r = Arc::new(ProxyRotator::new(&Some(proxies)));

        let handles: Vec<_> = (0..10)
            .map(|_| {
                let r = r.clone();
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        let addr = r.next_cdp().unwrap();
                        assert!(addr == "http://p1" || addr == "http://p2" || addr == "http://p3");
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    // ---- Binary Content-Type Detection ----

    #[test]
    fn test_is_binary_content_type_images() {
        assert!(is_binary_content_type("image/png"));
        assert!(is_binary_content_type("image/jpeg"));
        assert!(is_binary_content_type("image/webp"));
        assert!(is_binary_content_type("image/svg+xml"));
        assert!(is_binary_content_type("image/gif"));
    }

    #[test]
    fn test_is_binary_content_type_with_charset() {
        // Content-Type values often include charset or other params.
        assert!(is_binary_content_type("image/png; charset=utf-8"));
        assert!(is_binary_content_type(
            "application/pdf; boundary=something"
        ));
        assert!(is_binary_content_type("font/woff2; charset=binary"));
    }

    #[test]
    fn test_is_binary_content_type_fonts() {
        assert!(is_binary_content_type("font/woff"));
        assert!(is_binary_content_type("font/woff2"));
        assert!(is_binary_content_type("font/ttf"));
        assert!(is_binary_content_type("application/vnd.ms-fontobject"));
        assert!(is_binary_content_type("application/x-font-ttf"));
        assert!(is_binary_content_type("application/x-font-woff"));
    }

    #[test]
    fn test_is_binary_content_type_archives() {
        assert!(is_binary_content_type("application/pdf"));
        assert!(is_binary_content_type("application/zip"));
        assert!(is_binary_content_type("application/gzip"));
        assert!(is_binary_content_type("application/x-gzip"));
        assert!(is_binary_content_type("application/octet-stream"));
        assert!(is_binary_content_type("application/wasm"));
        assert!(is_binary_content_type("application/x-tar"));
        assert!(is_binary_content_type("application/x-bzip2"));
        assert!(is_binary_content_type("application/x-7z-compressed"));
        assert!(is_binary_content_type("application/x-rar-compressed"));
    }

    #[test]
    fn test_is_binary_content_type_audio_video() {
        assert!(is_binary_content_type("audio/mpeg"));
        assert!(is_binary_content_type("audio/ogg"));
        assert!(is_binary_content_type("video/mp4"));
        assert!(is_binary_content_type("video/webm"));
    }

    #[test]
    fn test_is_binary_content_type_html_not_binary() {
        assert!(!is_binary_content_type("text/html"));
        assert!(!is_binary_content_type("text/html; charset=utf-8"));
        assert!(!is_binary_content_type("text/plain"));
        assert!(!is_binary_content_type("application/json"));
        assert!(!is_binary_content_type("application/javascript"));
        assert!(!is_binary_content_type("text/css"));
        assert!(!is_binary_content_type("application/xml"));
    }

    // ---- Asset URL Skip ----

    #[test]
    fn test_should_skip_backend_for_asset_urls() {
        assert!(should_skip_backend_for_url(
            "https://example.com/photo.jpg",
            &[]
        ));
        assert!(should_skip_backend_for_url(
            "https://example.com/photo.png",
            &[]
        ));
        assert!(should_skip_backend_for_url(
            "https://example.com/font.woff2",
            &[]
        ));
        assert!(should_skip_backend_for_url(
            "https://example.com/doc.pdf",
            &[]
        ));
        assert!(should_skip_backend_for_url(
            "https://example.com/video.mp4",
            &[]
        ));
    }

    #[test]
    fn test_should_not_skip_backend_for_html_urls() {
        assert!(!should_skip_backend_for_url(
            "https://example.com/page.html",
            &[]
        ));
        assert!(!should_skip_backend_for_url(
            "https://example.com/about",
            &[]
        ));
        assert!(!should_skip_backend_for_url(
            "https://example.com/api/data",
            &[]
        ));
        assert!(!should_skip_backend_for_url("https://example.com/", &[]));
    }

    #[test]
    fn test_should_skip_backend_custom_extensions() {
        let extras = vec![
            crate::compact_str::CompactString::from("xml"),
            crate::compact_str::CompactString::from("rss"),
        ];
        assert!(should_skip_backend_for_url(
            "https://example.com/feed.xml",
            &extras
        ));
        assert!(should_skip_backend_for_url(
            "https://example.com/feed.rss",
            &extras
        ));
        assert!(should_skip_backend_for_url(
            "https://example.com/feed.RSS",
            &extras
        ));
        assert!(!should_skip_backend_for_url(
            "https://example.com/page.html",
            &extras
        ));
    }
}
