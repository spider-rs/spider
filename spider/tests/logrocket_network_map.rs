//! Inventory the Chrome network map for https://logrocket.com/careers and
//! build a candidate list of URLs that are NOT currently blocked by
//! `spider_network_blocker`'s ignore tries — the input for growing our
//! shared blocklist.
//!
//! Intentionally does NOT enable chrome interception on the CDP session:
//! we want to see every URL Chrome asks for and then *classify* each against
//! the tries ourselves. That tells us exactly which tracking / analytics /
//! ad origins are slipping through the current blocklist (the ones we'd
//! gain bytes by adding).
//!
//! The test skips silently when no Chrome is listening on 127.0.0.1:9222.
//!
//! Run:
//!   CHROME_URL=ws://127.0.0.1:9222 \
//!     cargo test -p spider --test logrocket_network_map \
//!     --features "chrome serde" -- --nocapture
//!
//! `serde` is required because `spider_transformations` (used for the
//! markdown snapshot) pulls it in transitively.

#[cfg(feature = "chrome")]
mod net_map {
    use spider::chromiumoxide::cdp::browser_protocol::network::{
        EnableParams, EventLoadingFailed, EventLoadingFinished, EventRequestWillBeSent,
    };
    use spider::chromiumoxide::cdp::browser_protocol::target::CreateTargetParams;
    use spider::chromiumoxide::Browser;
    use spider::tokio;
    use spider::tokio_stream::StreamExt;
    use spider_network_blocker::scripts::{
        URL_IGNORE_EMBEDED_TRIE, URL_IGNORE_SCRIPT_BASE_PATHS, URL_IGNORE_TRIE,
        URL_IGNORE_TRIE_PATHS,
    };
    use spider_network_blocker::xhr::{URL_IGNORE_XHR_MEDIA_TRIE, URL_IGNORE_XHR_TRIE};
    use std::collections::HashMap;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const TARGET_URL: &str = "https://logrocket.com/careers";
    /// Origins considered first-party for reporting purposes (stripped from
    /// candidates even if they'd otherwise match nothing in the tries).
    const PRIMARY_HOST_SUFFIXES: &[&str] = &["logrocket.com", "logrocket.io"];
    /// Relative to CARGO_MANIFEST_DIR (the `spider` crate dir).
    const OUTPUT_PATH: &str = "tests/fixtures/network_blacklist_candidates_logrocket.txt";
    /// Markdown snapshot of the rendered page — baseline input for a
    /// before/after diff when adding entries to URL_IGNORE_TRIE.
    const MARKDOWN_PATH: &str = "tests/fixtures/logrocket_careers_markdown.md";
    const NAV_TIMEOUT_SECS: u64 = 45;
    /// Stop capturing once the page has been quiet this long.
    const IDLE_QUIET_MS: u64 = 4000;

    async fn chrome_url() -> Option<String> {
        if std::net::TcpStream::connect("127.0.0.1:9222").is_err() {
            return None;
        }
        let resp = spider::reqwest::get("http://127.0.0.1:9222/json/version")
            .await
            .ok()?;
        let json: serde_json::Value = resp.json().await.ok()?;
        json["webSocketDebuggerUrl"].as_str().map(String::from)
    }

    fn host_of(url: &str) -> Option<String> {
        spider::url::Url::parse(url)
            .ok()?
            .host_str()
            .map(str::to_string)
    }

    /// Returns the name of the first trie that would block `url`, or `None`.
    /// Mirrors the check order used by chromey's intercept path so the
    /// "already blocked" bucket reflects production behavior.
    fn blocked_by_trie(url: &str) -> Option<&'static str> {
        if URL_IGNORE_TRIE.contains_prefix(url) {
            return Some("URL_IGNORE_TRIE");
        }
        if URL_IGNORE_XHR_TRIE.contains_prefix(url) {
            return Some("URL_IGNORE_XHR_TRIE");
        }
        if URL_IGNORE_XHR_MEDIA_TRIE.contains_prefix(url) {
            return Some("URL_IGNORE_XHR_MEDIA_TRIE");
        }
        if URL_IGNORE_EMBEDED_TRIE.contains_prefix(url) {
            return Some("URL_IGNORE_EMBEDED_TRIE");
        }
        if let Ok(u) = spider::url::Url::parse(url) {
            let p = u.path();
            if URL_IGNORE_TRIE_PATHS.contains_prefix(p) {
                return Some("URL_IGNORE_TRIE_PATHS");
            }
            if URL_IGNORE_SCRIPT_BASE_PATHS.contains_prefix(p) {
                return Some("URL_IGNORE_SCRIPT_BASE_PATHS");
            }
        }
        None
    }

    #[derive(Default, Clone, Debug)]
    struct Record {
        url: String,
        resource_type: String,
        bytes: u64,
        failed: bool,
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn logrocket_network_map() {
        let Some(ws) = chrome_url().await else {
            eprintln!("SKIP: no Chrome at 127.0.0.1:9222");
            return;
        };

        let (browser, mut handler) = Browser::connect(ws).await.expect("connect chrome");
        let handler_task =
            tokio::task::spawn(async move { while handler.next().await.is_some() {} });

        let page = browser
            .new_page(CreateTargetParams::new("about:blank"))
            .await
            .expect("new page");
        page.execute(EnableParams::default())
            .await
            .expect("Network.enable");

        let mut req_events = page
            .event_listener::<EventRequestWillBeSent>()
            .await
            .expect("sub requestWillBeSent");
        let mut fin_events = page
            .event_listener::<EventLoadingFinished>()
            .await
            .expect("sub loadingFinished");
        let mut fail_events = page
            .event_listener::<EventLoadingFailed>()
            .await
            .expect("sub loadingFailed");

        let records: Arc<Mutex<HashMap<String, Record>>> = Arc::new(Mutex::new(HashMap::new()));
        let last_activity = Arc::new(Mutex::new(Instant::now()));

        // RequestWillBeSent — register URL + resource type by request_id.
        {
            let r = records.clone();
            let la = last_activity.clone();
            tokio::spawn(async move {
                while let Some(ev) = req_events.next().await {
                    let rid: &str = ev.request_id.as_ref();
                    let mut map = r.lock().unwrap();
                    let rec = map.entry(rid.to_string()).or_default();
                    // First url wins; redirects keep updating request.url but we
                    // want a single entry per request_id with the latest URL.
                    rec.url.clone_from(&ev.request.url);
                    rec.resource_type = ev
                        .r#type
                        .as_ref()
                        .map(|t| format!("{:?}", t))
                        .unwrap_or_else(|| "Other".into());
                    *la.lock().unwrap() = Instant::now();
                }
            });
        }

        // LoadingFinished — record encoded bytes.
        {
            let r = records.clone();
            let la = last_activity.clone();
            tokio::spawn(async move {
                while let Some(ev) = fin_events.next().await {
                    let rid: &str = ev.request_id.as_ref();
                    let mut map = r.lock().unwrap();
                    if let Some(rec) = map.get_mut(rid) {
                        rec.bytes = ev.encoded_data_length.max(0.0) as u64;
                    }
                    *la.lock().unwrap() = Instant::now();
                }
            });
        }

        // LoadingFailed — mark request as failed (still counts its URL in the map).
        {
            let r = records.clone();
            let la = last_activity.clone();
            tokio::spawn(async move {
                while let Some(ev) = fail_events.next().await {
                    let rid: &str = ev.request_id.as_ref();
                    let mut map = r.lock().unwrap();
                    if let Some(rec) = map.get_mut(rid) {
                        rec.failed = true;
                    }
                    *la.lock().unwrap() = Instant::now();
                }
            });
        }

        // Navigate with a hard cap so a hang never blocks the test.
        let navigation =
            tokio::time::timeout(Duration::from_secs(NAV_TIMEOUT_SECS), page.goto(TARGET_URL))
                .await;
        match navigation {
            Ok(Ok(_)) => eprintln!("navigation completed"),
            Ok(Err(e)) => eprintln!("navigation error (continuing to capture): {e}"),
            Err(_) => eprintln!("navigation timed out (continuing to capture)"),
        }

        // Capture until the page has been quiet for `IDLE_QUIET_MS`, capped by
        // `NAV_TIMEOUT_SECS` overall so the test always terminates.
        let hard_deadline = Instant::now() + Duration::from_secs(NAV_TIMEOUT_SECS);
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let idle = last_activity.lock().unwrap().elapsed();
            if idle.as_millis() >= IDLE_QUIET_MS as u128 {
                eprintln!("idle for {:?} — stopping capture", idle);
                break;
            }
            if Instant::now() >= hard_deadline {
                eprintln!("hit hard deadline — stopping capture");
                break;
            }
        }

        // Classify & summarize.
        let records: Vec<Record> = records.lock().unwrap().values().cloned().collect();
        let mut by_origin: HashMap<String, (u64, usize, Vec<Record>)> = HashMap::new();
        let mut total_bytes: u64 = 0;
        let mut primary_bytes: u64 = 0;
        let mut primary_count: usize = 0;
        let mut blocked_bytes: u64 = 0;
        let mut blocked_count: usize = 0;
        let mut candidate_bytes: u64 = 0;
        let mut candidate_count: usize = 0;
        let mut blocked_by_category: HashMap<&'static str, (u64, usize)> = HashMap::new();

        for rec in &records {
            if rec.url.is_empty() {
                continue;
            }
            total_bytes = total_bytes.saturating_add(rec.bytes);

            let host = host_of(&rec.url).unwrap_or_else(|| "<?>".into());
            let is_primary = PRIMARY_HOST_SUFFIXES
                .iter()
                .any(|s| host == *s || host.ends_with(&format!(".{s}")));
            if is_primary {
                primary_bytes = primary_bytes.saturating_add(rec.bytes);
                primary_count += 1;
                continue;
            }

            if let Some(trie) = blocked_by_trie(&rec.url) {
                blocked_bytes = blocked_bytes.saturating_add(rec.bytes);
                blocked_count += 1;
                let entry = blocked_by_category.entry(trie).or_default();
                entry.0 = entry.0.saturating_add(rec.bytes);
                entry.1 += 1;
                continue;
            }

            candidate_bytes = candidate_bytes.saturating_add(rec.bytes);
            candidate_count += 1;
            let entry = by_origin.entry(host).or_default();
            entry.0 = entry.0.saturating_add(rec.bytes);
            entry.1 += 1;
            entry.2.push(rec.clone());
        }

        let mut origins: Vec<(String, u64, usize, Vec<Record>)> = by_origin
            .into_iter()
            .map(|(h, (b, c, r))| (h, b, c, r))
            .collect();
        origins.sort_by(|a, b| b.1.cmp(&a.1));

        // Write the candidate file under CARGO_MANIFEST_DIR/tests/fixtures/.
        let out = {
            let manifest =
                std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo test");
            std::path::Path::new(&manifest).join(OUTPUT_PATH)
        };
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).expect("mkdir fixtures");
        }
        let mut file = std::fs::File::create(&out).expect("create candidate file");
        writeln!(file, "# Candidate blocklist entries for {TARGET_URL}").unwrap();
        writeln!(
            file,
            "# Generated by tests/logrocket_network_map.rs; sorted by origin bytes desc."
        )
        .unwrap();
        writeln!(
            file,
            "# Not currently matched by URL_IGNORE_TRIE, URL_IGNORE_XHR_TRIE,"
        )
        .unwrap();
        writeln!(
            file,
            "# URL_IGNORE_XHR_MEDIA_TRIE, URL_IGNORE_EMBEDED_TRIE, URL_IGNORE_TRIE_PATHS,"
        )
        .unwrap();
        writeln!(file, "# or URL_IGNORE_SCRIPT_BASE_PATHS.").unwrap();
        writeln!(file, "#").unwrap();
        writeln!(
            file,
            "# Totals: {total_bytes} bytes observed | {primary_bytes} primary | \
             {blocked_bytes} already-blocked | {candidate_bytes} candidate ({candidate_count} URLs across {} origins)",
            origins.len()
        )
        .unwrap();
        writeln!(file).unwrap();
        for (host, bytes, count, recs) in &origins {
            writeln!(file, "# {bytes} bytes | {count} requests | {host}").unwrap();
            for rec in recs.iter().take(5) {
                writeln!(file, "{} [{}]", rec.url, rec.resource_type).unwrap();
            }
            if recs.len() > 5 {
                writeln!(file, "# ...+{} more URLs on {host}", recs.len() - 5).unwrap();
            }
            writeln!(file).unwrap();
        }

        // Stderr summary.
        eprintln!();
        eprintln!("==== network map for {TARGET_URL} ====");
        eprintln!(
            "{:>10} bytes total          ({} records)",
            total_bytes,
            records.len()
        );
        eprintln!(
            "{:>10} bytes primary        ({primary_count} records)",
            primary_bytes
        );
        eprintln!(
            "{:>10} bytes already-blocked ({blocked_count} records)",
            blocked_bytes
        );
        eprintln!(
            "{:>10} bytes candidate      ({candidate_count} records across {} origins)",
            candidate_bytes,
            origins.len()
        );

        if !blocked_by_category.is_empty() {
            eprintln!();
            eprintln!("Already-blocked breakdown:");
            let mut cats: Vec<_> = blocked_by_category.iter().collect();
            cats.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
            for (cat, (b, c)) in cats {
                eprintln!("  {:>10} bytes  {:>4} req  {cat}", b, c);
            }
        }

        eprintln!();
        eprintln!("Top candidate origins by bytes:");
        for (host, bytes, count, _) in origins.iter().take(20) {
            eprintln!("  {:>10} bytes  {:>4} req  {host}", bytes, count);
        }
        eprintln!();
        eprintln!("Candidate list written to {}", out.display());

        // Grab the rendered HTML and snapshot it as markdown so before/after
        // runs can be diffed to confirm blocklist additions don't remove real
        // page content. Readability-stripped markdown is stable across
        // tracker-only changes.
        let markdown = match page.content().await {
            Ok(html) => {
                let md = spider_transformations::transformation::content::transform_markdown(
                    &html, false,
                );
                eprintln!(
                    "markdown: {} bytes ({} lines) — full snapshot at {}",
                    md.len(),
                    md.lines().count(),
                    OUTPUT_PATH_MARKDOWN_DISPLAY,
                );
                Some(md)
            }
            Err(e) => {
                eprintln!("warn: failed to read page.content() for markdown snapshot: {e}");
                None
            }
        };
        if let Some(md) = markdown {
            let manifest =
                std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo test");
            let md_path = std::path::Path::new(&manifest).join(MARKDOWN_PATH);
            if let Some(parent) = md_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            if let Err(e) = std::fs::write(&md_path, md.as_bytes()) {
                eprintln!("warn: failed to write markdown snapshot: {e}");
            } else {
                eprintln!("markdown snapshot written to {}", md_path.display());
            }
        }

        // Require that we saw meaningful network activity — otherwise the
        // output file is empty and we should not silently pass.
        assert!(total_bytes > 0, "expected network activity, got 0 bytes");
        assert!(
            records.len() > 1,
            "expected multiple requests, got {}",
            records.len()
        );

        // Clean shutdown so the browser is left in a good state for the next run.
        drop(page);
        drop(browser);
        handler_task.abort();
    }

    // Short display string for the stderr summary; full path is logged on write.
    const OUTPUT_PATH_MARKDOWN_DISPLAY: &str = MARKDOWN_PATH;
}
