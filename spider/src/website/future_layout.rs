//! Future layout budgets for the crawl entry points.
//!
//! These tests measure `size_of` for the futures the crawl path builds,
//! without polling any of them. The number is the coroutine's state
//! machine size, which is what a `Box::pin` allocates or what a caller's
//! own state machine has to reserve inline. It is not a stack measurement:
//! poll and drop still run as native frames on the worker, so a small
//! future says nothing about stack safety on its own.
//!
//! The defect this guards against is a by-value future parameter moved
//! into a wrapper inside an `async fn`. Parameters become upvars of the
//! coroutine and live in its fixed prefix; the wrapper (for example
//! `tokio::time::Timeout<F>`) is a saved local. rustc never overlaps the
//! two, so the state machine reserves the payload twice. Passing the
//! payload already boxed keeps the parameter slot at pointer size.
//!
//! Run with `--nocapture` to print the full table.

use super::*;
use core::future::Future;
use core::mem::size_of;

/// Size of the future a closure would build. The closure is never
/// called, so nothing here needs a runtime, a client, or a browser.
fn future_size<F: Future>(_make: impl FnOnce() -> F) -> usize {
    size_of::<F>()
}

/// Placeholder argument for a closure that is never invoked.
#[allow(unreachable_code)]
fn never<T>() -> T {
    unreachable!("future_layout closures are sized, never called")
}

/// A payload future of a known, feature independent size, used to
/// measure the amplification factor of a wrapper in isolation.
async fn payload_64k<T>() -> T {
    let buf = [0u8; 64 * 1024];
    std::future::ready(()).await;
    std::hint::black_box(&buf);
    never()
}

/// Upper bound for the crawl timeout wrapper. It only holds a pinned
/// box, the timer state and two small arguments, so it must not scale
/// with the payload it wraps.
const CRAWL_TIMEOUT_WRAPPER_MAX_BYTES: usize = 1024;

/// Upper bound for a top level crawl entry point. Each one boxes its
/// crawl body, so what remains inline is `setup`, the cache shortcircuit
/// and a few locals.
const CRAWL_ENTRY_MAX_BYTES: usize = 16 * 1024;

fn print_rows(title: &str, rows: &[(&str, usize)]) {
    println!("=== {title} (bytes)");
    for (name, size) in rows {
        println!("{size:>9}  {name}");
    }
}

#[test]
fn crawl_timeout_wrapper_does_not_scale_with_its_payload() {
    let payload = future_size(|| payload_64k::<()>());
    let wrapped = future_size(|| run_with_crawl_timeout(None, "", Box::pin(payload_64k::<()>())));
    print_rows(
        "run_with_crawl_timeout",
        &[
            ("payload_64k", payload),
            ("run_with_crawl_timeout(Box::pin(payload_64k))", wrapped),
        ],
    );
    assert!(
        wrapped <= CRAWL_TIMEOUT_WRAPPER_MAX_BYTES,
        "run_with_crawl_timeout reserves {wrapped} bytes for a boxed payload; budget is {CRAWL_TIMEOUT_WRAPPER_MAX_BYTES}"
    );
}

#[test]
fn crawl_entry_points_box_their_bodies() {
    let mut rows: Vec<(&str, usize)> = vec![
        ("Website::crawl", future_size(|| Website::crawl(never()))),
        (
            "Website::crawl_raw",
            future_size(|| Website::crawl_raw(never())),
        ),
        (
            "Website::crawl_smart",
            future_size(|| Website::crawl_smart(never())),
        ),
        (
            "Website::crawl_sitemap",
            future_size(|| Website::crawl_sitemap(never())),
        ),
    ];
    #[cfg(not(feature = "decentralized"))]
    rows.push((
        "Website::crawl_raw_send",
        future_size(|| Website::crawl_raw_send(never(), never())),
    ));
    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    rows.push((
        "Website::crawl_chrome_send",
        future_size(|| Website::crawl_chrome_send(never(), never())),
    ));
    #[cfg(all(
        feature = "sitemap",
        feature = "chrome",
        not(feature = "decentralized")
    ))]
    rows.push((
        "Website::crawl_sitemap_chrome",
        future_size(|| Website::crawl_sitemap_chrome(never())),
    ));

    // The body `crawl` hands to the timeout wrapper, sized on its own.
    // It is expected to dwarf the entry point that boxes it.
    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    let crawl_body = future_size(|| async {
        let w: &mut Website = never();
        let c: &Client = never();
        let h: &Option<Arc<AtomicI8>> = never();
        w.crawl_concurrent(c, h).await;
        w.sitemap_crawl_chain(c, h, false).await;
    });
    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    rows.push((
        "crawl body {crawl_concurrent; sitemap_crawl_chain}",
        crawl_body,
    ));

    print_rows("crawl entry points", &rows);

    for (name, size) in &rows {
        if name.starts_with("Website::crawl") {
            assert!(
                *size <= CRAWL_ENTRY_MAX_BYTES,
                "{name} future is {size} bytes; budget is {CRAWL_ENTRY_MAX_BYTES}"
            );
        }
    }
    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    {
        let crawl = rows[0].1;
        assert!(
            crawl < crawl_body,
            "Website::crawl ({crawl}) should be smaller than the body it boxes ({crawl_body}); the body is inline again"
        );
    }
}
