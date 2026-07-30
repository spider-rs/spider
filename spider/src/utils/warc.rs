//! WARC 1.1 writer for web archive output.
//!
//! Feature-gated behind `warc`. Produces spec-compliant WARC/1.1 files
//! from crawled pages. Zero external WARC dependencies — writes the format
//! directly.
//!
//! **Fully lock-free**: callers serialize records into `Vec<u8>` and send
//! them through an unbounded MPSC channel. A single background task drains
//! the channel and writes to a `BufWriter<File>`. No mutexes, no contention
//! on the hot path.
//!
//! Reference: <https://iipc.github.io/warc-specifications/specifications/warc-format/warc-1.1/>

#[cfg(feature = "warc")]
mod inner {
    use crate::page::Page;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use tokio::sync::{broadcast, mpsc};

    /// CRLF constant.
    const CRLF: &[u8] = b"\r\n";

    /// Default buffer size for the WARC file writer (256 KB).
    const BUF_SIZE: usize = 256 * 1024;

    /// Default soft ceiling on serialized-but-unwritten WARC bytes (64 MiB).
    const DEFAULT_QUEUE_SOFT_LIMIT: usize = 64 * 1024 * 1024;

    lazy_static::lazy_static! {
        /// Soft ceiling on bytes queued for the WARC file writer, via
        /// `SPIDER_WARC_QUEUE_BYTES`. A crawl can serialize records far faster than a
        /// disk drains them, and the channel itself is unbounded, so without this the
        /// backlog is bounded only by RAM. `0` disables the backpressure entirely
        /// (the historical behaviour). A malformed value falls back to the default
        /// rather than panicking.
        static ref QUEUE_SOFT_LIMIT: usize = std::env::var("SPIDER_WARC_QUEUE_BYTES")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(DEFAULT_QUEUE_SOFT_LIMIT);
    }

    /// A unit of work for the background file writer.
    ///
    /// Private: `WarcWriter::create`'s public signature is unchanged, so adding the
    /// flush variant is not a breaking change.
    enum WarcOp {
        /// Pre-serialized record bytes to append.
        Bytes(Vec<u8>),
        /// Flush the `BufWriter` and acknowledge, so callers get a deterministic
        /// durability point and can observe I/O errors.
        Flush(tokio::sync::oneshot::Sender<io::Result<()>>),
    }

    /// Configuration for WARC output.
    #[derive(Debug, Clone, PartialEq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct WarcConfig {
        /// Output file path. Defaults to `"output.warc"`.
        pub path: String,
        /// Whether to include a `warcinfo` record at the start. Default: true.
        pub write_warcinfo: bool,
        /// Software identifier for the `warcinfo` record.
        pub software: String,
    }

    impl Default for WarcConfig {
        fn default() -> Self {
            Self {
                path: "output.warc".to_string(),
                write_warcinfo: true,
                software: format!("spider/{}", env!("CARGO_PKG_VERSION")),
            }
        }
    }

    /// Generate a UUID v4-style string for WARC record IDs.
    /// Format: `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`
    fn generate_uuid() -> String {
        let mut buf = [0u8; 16];
        #[cfg(feature = "spoof")]
        {
            for b in buf.iter_mut() {
                *b = fastrand::u8(..);
            }
        }
        #[cfg(not(feature = "spoof"))]
        {
            use std::hash::{Hash, Hasher};
            static CTR: AtomicU64 = AtomicU64::new(0);
            let mut hasher = ahash::AHasher::default();
            CTR.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
            std::thread::current().id().hash(&mut hasher);
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            ts.hash(&mut hasher);
            let h1 = hasher.finish();
            h1.hash(&mut hasher);
            let h2 = hasher.finish();
            buf[..8].copy_from_slice(&h1.to_le_bytes());
            buf[8..].copy_from_slice(&h2.to_le_bytes());
        }
        // Set version (4) and variant (RFC 4122).
        buf[6] = (buf[6] & 0x0f) | 0x40;
        buf[8] = (buf[8] & 0x3f) | 0x80;

        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            buf[0], buf[1], buf[2], buf[3],
            buf[4], buf[5],
            buf[6], buf[7],
            buf[8], buf[9],
            buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        )
    }

    /// Generate WARC-Date in ISO8601 UTC format: `YYYY-MM-DDThh:mm:ssZ`.
    fn warc_date_now() -> String {
        let now = std::time::SystemTime::now();
        let dur = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = dur.as_secs();

        let days = secs / 86400;
        let time_of_day = secs % 86400;
        let hours = time_of_day / 3600;
        let minutes = (time_of_day % 3600) / 60;
        let seconds = time_of_day % 60;

        let (year, month, day) = days_to_ymd(days);

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            year, month, day, hours, minutes, seconds,
        )
    }

    /// Convert days since Unix epoch to (year, month, day).
    fn days_to_ymd(days: u64) -> (u64, u64, u64) {
        let days = days + 719_468;
        let era = days / 146_097;
        let doe = days - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        (y, m, d)
    }

    /// Append WARC record headers to a buffer (everything before the payload).
    fn append_warc_header(
        buf: &mut Vec<u8>,
        record_type: &str,
        record_id: &str,
        date: &str,
        target_uri: Option<&str>,
        content_type: &str,
        content_length: usize,
        #[cfg(feature = "remote_addr")] ip_address: Option<&str>,
    ) {
        let _ = write!(buf, "WARC/1.1\r\n");
        let _ = write!(buf, "WARC-Type: {record_type}\r\n");
        let _ = write!(buf, "WARC-Record-ID: <urn:uuid:{record_id}>\r\n");
        let _ = write!(buf, "WARC-Date: {date}\r\n");
        if let Some(uri) = target_uri {
            let _ = write!(buf, "WARC-Target-URI: {uri}\r\n");
        }
        #[cfg(feature = "remote_addr")]
        if let Some(ip) = ip_address {
            let _ = write!(buf, "WARC-IP-Address: {ip}\r\n");
        }
        let _ = write!(buf, "Content-Type: {content_type}\r\n");
        let _ = write!(buf, "Content-Length: {content_length}\r\n");
        buf.extend_from_slice(CRLF);
    }

    /// Serialize a `warcinfo` record into a self-contained byte buffer.
    fn serialize_warcinfo(software: &str) -> Vec<u8> {
        let record_id = generate_uuid();
        let date = warc_date_now();
        let payload = format!("software: {software}\r\nformat: WARC File Format 1.1\r\n");
        let payload_bytes = payload.as_bytes();

        let mut buf = Vec::with_capacity(256 + payload_bytes.len());
        append_warc_header(
            &mut buf,
            "warcinfo",
            &record_id,
            &date,
            None,
            "application/warc-fields",
            payload_bytes.len(),
            #[cfg(feature = "remote_addr")]
            None,
        );
        buf.extend_from_slice(payload_bytes);
        buf.extend_from_slice(CRLF);
        buf.extend_from_slice(CRLF);
        buf
    }

    /// Guess a reasonable Content-Type from the first few bytes of a response body.
    /// Used when the captured headers don't contain one so WARC replay tools
    /// still know how to render the resource.
    fn guess_content_type(body: &[u8]) -> &'static str {
        let trimmed = {
            let mut i = 0;
            while i < body.len()
                && (body[i] == b' ' || body[i] == b'\t' || body[i] == b'\r' || body[i] == b'\n')
            {
                i += 1;
            }
            &body[i..]
        };
        if trimmed.is_empty() {
            return "application/octet-stream";
        }
        match trimmed[0] {
            b'<' => "text/html; charset=utf-8",
            b'{' | b'[' => "application/json; charset=utf-8",
            _ => {
                if trimmed.len() >= 4 && &trimmed[..4] == b"\x89PNG" {
                    "image/png"
                } else if trimmed.len() >= 3 && &trimmed[..3] == b"\xff\xd8\xff" {
                    "image/jpeg"
                } else if trimmed.len() >= 4 && &trimmed[..4] == b"GIF8" {
                    "image/gif"
                } else if trimmed.len() >= 4 && &trimmed[..4] == b"%PDF" {
                    "application/pdf"
                } else if trimmed.is_ascii() {
                    "text/plain; charset=utf-8"
                } else {
                    "application/octet-stream"
                }
            }
        }
    }

    /// Serialize a WARC `response` record from a `Page` into a self-contained byte buffer.
    /// Returns `None` if the page has an empty URL.
    pub fn serialize_page(page: &Page) -> Option<Vec<u8>> {
        let url = page.get_url();
        if url.is_empty() {
            return None;
        }

        let status = page.status_code.as_u16();
        let body = page.get_html_bytes_u8();

        // Reconstruct the HTTP response payload.
        let mut payload = Vec::with_capacity(512 + body.len());
        {
            let _ = write!(
                payload,
                "HTTP/1.1 {} {}\r\n",
                status,
                page.status_code.canonical_reason().unwrap_or("Unknown")
            );
        }

        // Track whether Content-Type / Content-Length are present in the captured
        // response headers. WARC replay tools (replayweb.page, pywb) require
        // Content-Type in the HTTP payload to know how to serve the resource.
        let mut has_content_type = false;
        let mut has_content_length = false;

        if let Some(ref headers) = page.headers {
            for (name, value) in headers.iter() {
                let n = name.as_str();
                if n.eq_ignore_ascii_case("content-type") {
                    has_content_type = true;
                } else if n.eq_ignore_ascii_case("content-length") {
                    has_content_length = true;
                }
                payload.extend_from_slice(n.as_bytes());
                payload.extend_from_slice(b": ");
                payload.extend_from_slice(value.as_bytes());
                payload.extend_from_slice(CRLF);
            }
        }

        // Synthesize a Content-Type when the captured headers don't have one so
        // replay tools can serve the resource. Guess from the body's first byte.
        if !has_content_type {
            let ct = guess_content_type(body);
            payload.extend_from_slice(b"Content-Type: ");
            payload.extend_from_slice(ct.as_bytes());
            payload.extend_from_slice(CRLF);
        }

        // Ensure Content-Length reflects the actual body length for the replay
        // engine even when upstream headers omitted it (e.g. chunked encoding).
        if !has_content_length {
            let _ = write!(payload, "Content-Length: {}\r\n", body.len());
        }

        payload.extend_from_slice(CRLF);
        payload.extend_from_slice(body);

        let record_id = generate_uuid();
        let date = warc_date_now();

        #[cfg(feature = "remote_addr")]
        let ip_str = page.remote_addr.as_ref().map(|a| a.ip().to_string());

        let mut buf = Vec::with_capacity(512 + payload.len());
        append_warc_header(
            &mut buf,
            "response",
            &record_id,
            &date,
            Some(url),
            "application/http; msgtype=response",
            payload.len(),
            #[cfg(feature = "remote_addr")]
            ip_str.as_deref(),
        );
        buf.extend_from_slice(&payload);
        buf.extend_from_slice(CRLF);
        buf.extend_from_slice(CRLF);

        Some(buf)
    }

    /// A lock-free WARC 1.1 file writer.
    ///
    /// Callers serialize records and send pre-built byte buffers through an
    /// unbounded MPSC channel. A single background task (spawned via
    /// [`WarcWriter::spawn_writer_task`]) drains the channel and writes
    /// sequentially to disk. Zero contention on the hot path.
    ///
    /// Safe to clone and share across tasks.
    #[derive(Clone)]
    pub struct WarcWriter {
        tx: mpsc::UnboundedSender<WarcOp>,
        record_count: std::sync::Arc<AtomicU64>,
        path: std::sync::Arc<PathBuf>,
        /// Bytes serialized but not yet written to disk. Drives the backpressure in
        /// [`spawn_warc_writer`].
        queued_bytes: std::sync::Arc<AtomicUsize>,
        /// Signalled by the writer task after each record lands, so a producer
        /// waiting on the soft limit wakes as soon as the backlog drains.
        drained: std::sync::Arc<tokio::sync::Notify>,
    }

    impl WarcWriter {
        /// Create a new WARC writer and spawn the background file-writing task.
        ///
        /// Returns `(writer, join_handle)`. The handle resolves when the writer
        /// is dropped (all senders closed) or an I/O error occurs.
        pub fn create(
            config: &WarcConfig,
        ) -> io::Result<(Self, tokio::task::JoinHandle<io::Result<()>>)> {
            let path = PathBuf::from(&config.path);

            // Ensure parent directory exists.
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }

            let file = std::fs::File::create(&path)?;
            let (tx, rx) = mpsc::unbounded_channel::<WarcOp>();
            let record_count = std::sync::Arc::new(AtomicU64::new(0));
            let queued_bytes = std::sync::Arc::new(AtomicUsize::new(0));
            let drained = std::sync::Arc::new(tokio::sync::Notify::new());

            let writer = Self {
                tx,
                record_count: record_count.clone(),
                path: std::sync::Arc::new(path),
                queued_bytes: queued_bytes.clone(),
                drained: drained.clone(),
            };

            // Send warcinfo as first record if requested.
            if config.write_warcinfo {
                let buf = serialize_warcinfo(&config.software);
                writer.enqueue(buf);
            }

            // Spawn the single file-writer task on a blocking thread.
            let handle = Self::spawn_writer_task(file, rx, queued_bytes, drained);

            Ok((writer, handle))
        }

        /// Spawn the background task that drains the channel and writes to disk.
        fn spawn_writer_task(
            file: std::fs::File,
            mut rx: mpsc::UnboundedReceiver<WarcOp>,
            queued_bytes: std::sync::Arc<AtomicUsize>,
            drained: std::sync::Arc<tokio::sync::Notify>,
        ) -> tokio::task::JoinHandle<io::Result<()>> {
            tokio::task::spawn_blocking(move || {
                let mut w = io::BufWriter::with_capacity(BUF_SIZE, file);
                // Use blocking recv via a small runtime-free loop.
                while let Some(op) = rx.blocking_recv() {
                    match op {
                        WarcOp::Bytes(buf) => {
                            let len = buf.len();
                            let res = w.write_all(&buf);

                            // Release the backlog accounting before propagating any
                            // error, so a producer parked on the soft limit can never
                            // be stranded by a failed write. `saturating_sub` keeps
                            // this sound even if the counters ever disagree.
                            let _ = queued_bytes.fetch_update(
                                Ordering::AcqRel,
                                Ordering::Relaxed,
                                |queued| Some(queued.saturating_sub(len)),
                            );
                            drained.notify_waiters();

                            res?;
                        }
                        WarcOp::Flush(ack) => match w.flush() {
                            Ok(()) => {
                                let _ = ack.send(Ok(()));
                            }
                            Err(e) => {
                                // `io::Error` isn't `Clone`, so rebuild an equivalent
                                // one for the caller and still fail the task.
                                let _ = ack.send(Err(io::Error::new(e.kind(), e.to_string())));
                                return Err(e);
                            }
                        },
                    }
                }
                w.flush()?;
                Ok(())
            })
        }

        /// Queue pre-serialized bytes and account for them in the backlog.
        fn enqueue(&self, buf: Vec<u8>) {
            let len = buf.len();

            // Account before sending: the writer task may drain the record before this
            // thread resumes, and an under-count would be observable as a stuck
            // producer, whereas a transient over-count only delays one send.
            self.queued_bytes.fetch_add(len, Ordering::AcqRel);

            if self.tx.send(WarcOp::Bytes(buf)).is_ok() {
                self.record_count.fetch_add(1, Ordering::Relaxed);
            } else {
                // Receiver gone — undo the accounting so waiters don't park forever.
                let _ =
                    self.queued_bytes
                        .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |queued| {
                            Some(queued.saturating_sub(len))
                        });
                self.drained.notify_waiters();
            }
        }

        /// Write a WARC `response` record from a crawled `Page`.
        ///
        /// Serializes the record on the caller's thread and sends the bytes
        /// through the channel — fully lock-free.
        ///
        /// This call never blocks and applies no backpressure; the queue is bounded
        /// only for the in-crate bridge in [`spawn_warc_writer`]. External callers
        /// driving this directly at high rate should watch [`Self::queued_bytes`].
        pub fn write_page(&self, page: &Page) {
            if let Some(buf) = serialize_page(page) {
                self.enqueue(buf);
            }
        }

        /// Bytes serialized but not yet written to disk.
        pub fn queued_bytes(&self) -> usize {
            self.queued_bytes.load(Ordering::Acquire)
        }

        /// Flush everything queued so far to disk and report the result.
        ///
        /// Without this the only flush happens when the last `WarcWriter` clone drops,
        /// so a long-lived or reused `Website` could leave up to `BUF_SIZE` of records
        /// unreadable on disk indefinitely, and I/O errors from the writer task were
        /// discarded entirely.
        ///
        /// Returns `Ok(())` if the writer task has already finished — its `BufWriter`
        /// is flushed on the way out, so there is nothing left owing.
        pub async fn flush(&self) -> io::Result<()> {
            let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();

            if self.tx.send(WarcOp::Flush(ack_tx)).is_err() {
                return Ok(());
            }

            ack_rx.await.unwrap_or(Ok(()))
        }

        /// Number of records written so far.
        pub fn record_count(&self) -> u64 {
            self.record_count.load(Ordering::Relaxed)
        }

        /// The output file path.
        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    /// Spawn a background task that reads pages from a broadcast receiver
    /// and writes them to a WARC file via the lock-free writer.
    ///
    /// Returns a `JoinHandle` that resolves when the broadcast channel closes.
    /// The handle returns the total number of records written.
    ///
    /// Applies backpressure: if the writer falls more than `SPIDER_WARC_QUEUE_BYTES`
    /// behind, this task waits for the backlog to halve before pulling more pages,
    /// rather than letting the unbounded channel grow to the size of the crawl. No
    /// records are dropped — the broadcast ring upstream is what sheds under lag.
    pub fn spawn_warc_writer(
        writer: WarcWriter,
        mut rx: broadcast::Receiver<Page>,
    ) -> tokio::task::JoinHandle<u64> {
        tokio::task::spawn(async move {
            let soft_limit = *QUEUE_SOFT_LIMIT;
            let resume_at = soft_limit / 2;

            loop {
                match rx.recv().await {
                    Ok(page) => {
                        if page.blocked_crawl {
                            continue;
                        }
                        writer.write_page(&page);

                        if soft_limit > 0 && writer.queued_bytes() > soft_limit {
                            // Wait for the writer to drain to the low-water mark.
                            //
                            // The timeout is load-bearing, not belt-and-braces:
                            // `notify_waiters` only wakes tasks already parked, so a
                            // notification landing between the check and the await
                            // would otherwise be missed and park this task forever.
                            // Re-checking on a short timer makes that unmissable, so
                            // this loop cannot deadlock even if the writer task dies.
                            while writer.queued_bytes() > resume_at {
                                let _ = tokio::time::timeout(
                                    std::time::Duration::from_millis(50),
                                    writer.drained.notified(),
                                )
                                .await;

                                if writer.tx.is_closed() {
                                    break;
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        #[cfg(feature = "tracing")]
                        tracing::warn!("WARC writer lagged, skipped {n} pages");
                        let _ = n;
                        continue;
                    }
                }
            }
            writer.record_count()
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use reqwest::StatusCode;

        /// Helper to create a minimal Page for testing.
        fn make_test_page(url: &str, status: u16, body: &str) -> Page {
            let mut page = Page::default();
            page.url = url.to_string();
            page.status_code = StatusCode::from_u16(status).unwrap_or_default();
            page.html = Some(bytes::Bytes::from(body.to_owned()));
            page
        }

        /// A record must be readable on disk after an explicit `flush()`, *without*
        /// dropping the writer. Before `flush` existed the only flush happened when
        /// the last clone dropped, so a long-lived `Website` could leave up to
        /// `BUF_SIZE` of records unreadable on disk indefinitely.
        #[tokio::test]
        async fn flush_makes_records_readable_without_dropping_the_writer() {
            let dir = std::env::temp_dir().join(format!("spider_warc_flush_{}", generate_uuid()));
            let path = dir.join("out.warc");

            let config = WarcConfig {
                path: path.to_string_lossy().into_owned(),
                write_warcinfo: false,
                software: "spider-test".into(),
            };

            let (writer, _handle) = WarcWriter::create(&config).expect("writer creates");

            writer.write_page(&make_test_page(
                "https://example.com",
                200,
                "<html>flushed</html>",
            ));

            writer.flush().await.expect("flush succeeds");

            // Deliberately still holding `writer` — nothing has been dropped.
            let on_disk = std::fs::read(&path).expect("warc file readable");
            let content = String::from_utf8_lossy(&on_disk);

            assert!(
                content.contains("WARC-Target-URI: https://example.com"),
                "record should be on disk after flush, got {} bytes",
                on_disk.len()
            );
            assert!(content.contains("flushed"), "payload should be on disk");

            drop(writer);
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// The backlog accounting must return to zero once the writer drains, `flush`
        /// must be idempotent, and dropping every writer clone must let the task
        /// finish with all records on disk.
        #[tokio::test]
        async fn queued_bytes_drains_and_flush_is_idempotent() {
            let dir = std::env::temp_dir().join(format!("spider_warc_drain_{}", generate_uuid()));
            let path = dir.join("out.warc");

            let config = WarcConfig {
                path: path.to_string_lossy().into_owned(),
                write_warcinfo: true,
                software: "spider-test".into(),
            };

            let (writer, handle) = WarcWriter::create(&config).expect("writer creates");

            for i in 0..64 {
                writer.write_page(&make_test_page(
                    &format!("https://example.com/{i}"),
                    200,
                    "<html>body</html>",
                ));
            }

            writer.flush().await.expect("flush succeeds");

            assert_eq!(
                writer.queued_bytes(),
                0,
                "backlog accounting must return to zero once drained"
            );
            assert_eq!(
                writer.record_count(),
                65,
                "64 pages plus the warcinfo record"
            );

            // A second flush with nothing queued must still complete promptly.
            let again =
                tokio::time::timeout(std::time::Duration::from_secs(5), writer.flush()).await;
            assert!(
                matches!(again, Ok(Ok(()))),
                "flush must be idempotent and must not hang, got {again:?}"
            );

            // Dropping every clone closes the channel, so the writer task drains,
            // flushes and exits. The task ends only once *all* senders are gone —
            // holding any clone across this await would block it forever.
            drop(writer);
            let joined = tokio::time::timeout(std::time::Duration::from_secs(30), handle).await;

            match joined {
                Ok(Ok(Ok(()))) => {}
                other => panic!("writer task should finish cleanly, got {other:?}"),
            }

            let on_disk = std::fs::read(&path).expect("warc file readable");
            let content = String::from_utf8_lossy(&on_disk);
            assert_eq!(
                content.matches("WARC-Type: response").count(),
                64,
                "every response record should be on disk"
            );

            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn warc_date_format_is_valid() {
            let date = warc_date_now();
            assert_eq!(date.len(), 20, "date = {date}");
            assert!(date.ends_with('Z'));
            assert_eq!(&date[4..5], "-");
            assert_eq!(&date[7..8], "-");
            assert_eq!(&date[10..11], "T");
            assert_eq!(&date[13..14], ":");
            assert_eq!(&date[16..17], ":");
        }

        #[test]
        fn uuid_format_is_valid() {
            let uuid = generate_uuid();
            assert_eq!(uuid.len(), 36);
            assert_eq!(&uuid[8..9], "-");
            assert_eq!(&uuid[13..14], "-");
            assert_eq!(&uuid[18..19], "-");
            assert_eq!(&uuid[23..24], "-");
            assert_eq!(&uuid[14..15], "4");
        }

        #[test]
        fn uuid_uniqueness() {
            let mut set = hashbrown::HashSet::new();
            for _ in 0..1000 {
                assert!(set.insert(generate_uuid()), "duplicate UUID generated");
            }
        }

        #[test]
        fn days_to_ymd_epoch() {
            let (y, m, d) = days_to_ymd(0);
            assert_eq!((y, m, d), (1970, 1, 1));
        }

        #[test]
        fn days_to_ymd_known_date() {
            let (y, m, d) = days_to_ymd(20537);
            assert_eq!((y, m, d), (2026, 3, 25));
        }

        #[test]
        fn serialize_page_valid_record() {
            let page = make_test_page("https://example.com", 200, "<html>Hello</html>");
            let buf = serialize_page(&page).unwrap();
            let content = String::from_utf8_lossy(&buf);

            assert!(content.starts_with("WARC/1.1\r\n"));
            assert!(content.contains("WARC-Type: response\r\n"));
            assert!(content.contains("WARC-Target-URI: https://example.com\r\n"));
            assert!(content.contains("Content-Type: application/http; msgtype=response\r\n"));
            assert!(content.contains("HTTP/1.1 200 OK\r\n"));
            assert!(content.contains("<html>Hello</html>"));
            assert!(buf.ends_with(b"\r\n\r\n"));
        }

        #[test]
        fn serialize_page_empty_url_returns_none() {
            let page = make_test_page("", 200, "body");
            assert!(serialize_page(&page).is_none());
        }

        #[test]
        fn serialize_page_404() {
            let page = make_test_page("https://example.com/missing", 404, "Not Found");
            let buf = serialize_page(&page).unwrap();
            let content = String::from_utf8_lossy(&buf);
            assert!(content.contains("HTTP/1.1 404 Not Found\r\n"));
        }

        #[test]
        fn serialize_page_with_headers() {
            let mut page = make_test_page("https://example.com", 200, "<html>Hi</html>");
            let mut hdr_map = reqwest::header::HeaderMap::new();
            hdr_map.insert("content-type", "text/html; charset=utf-8".parse().unwrap());
            hdr_map.insert("x-custom", "test-value".parse().unwrap());
            page.headers = Some(hdr_map);

            let buf = serialize_page(&page).unwrap();
            let content = String::from_utf8_lossy(&buf);
            assert!(content.contains("content-type: text/html; charset=utf-8\r\n"));
            assert!(content.contains("x-custom: test-value\r\n"));
        }

        #[test]
        fn content_length_is_accurate() {
            let page = make_test_page("https://example.com", 200, "Hello");
            let buf = serialize_page(&page).unwrap();
            let content = String::from_utf8_lossy(&buf);

            let cl_line = content
                .lines()
                .find(|l| l.starts_with("Content-Length:"))
                .unwrap();
            let cl: usize = cl_line
                .split(':')
                .nth(1)
                .unwrap()
                .trim()
                .trim_end_matches('\r')
                .parse()
                .unwrap();

            let header_end = content.find("\r\n\r\n").unwrap() + 4;
            let payload = &buf[header_end..header_end + cl];
            assert_eq!(payload.len(), cl);
            assert!(payload.starts_with(b"HTTP/1.1"));
        }

        #[test]
        fn record_terminator_is_double_crlf() {
            let page = make_test_page("https://example.com", 200, "Test");
            let buf = serialize_page(&page).unwrap();
            assert!(buf.ends_with(b"\r\n\r\n"));
        }

        #[test]
        fn serialize_warcinfo_is_valid() {
            let buf = serialize_warcinfo("spider-test/0.1");
            let content = String::from_utf8_lossy(&buf);
            assert!(content.starts_with("WARC/1.1\r\n"));
            assert!(content.contains("WARC-Type: warcinfo\r\n"));
            assert!(content.contains("software: spider-test/0.1\r\n"));
            assert!(content.contains("Content-Type: application/warc-fields\r\n"));
            assert!(buf.ends_with(b"\r\n\r\n"));
        }

        #[tokio::test]
        async fn writer_creates_valid_file() {
            let dir = std::env::temp_dir().join("spider_warc_test_file");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("test_file.warc");

            let config = WarcConfig {
                path: path.to_string_lossy().to_string(),
                write_warcinfo: true,
                software: "spider-test/0.1".to_string(),
            };

            let (writer, handle) = WarcWriter::create(&config).unwrap();

            for i in 0..10 {
                let page = make_test_page(
                    &format!("https://example.com/page/{i}"),
                    200,
                    &format!("<html>Page {i}</html>"),
                );
                writer.write_page(&page);
            }

            assert_eq!(writer.record_count(), 11); // 1 warcinfo + 10 response

            // Drop writer to close channel, then wait for file writer to finish.
            drop(writer);
            handle.await.unwrap().unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            let count = content.matches("WARC/1.1\r\n").count();
            assert_eq!(count, 11);

            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn writer_no_warcinfo() {
            let dir = std::env::temp_dir().join("spider_warc_test_no_info");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("test_no_info.warc");

            let config = WarcConfig {
                path: path.to_string_lossy().to_string(),
                write_warcinfo: false,
                software: "test".to_string(),
            };

            let (writer, handle) = WarcWriter::create(&config).unwrap();

            let page = make_test_page("https://example.com", 200, "Hello");
            writer.write_page(&page);
            assert_eq!(writer.record_count(), 1);

            drop(writer);
            handle.await.unwrap().unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            assert!(!content.contains("WARC-Type: warcinfo"));
            assert!(content.contains("WARC-Type: response"));

            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn concurrent_writes_no_panic() {
            let dir = std::env::temp_dir().join("spider_warc_test_concurrent");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("test_concurrent.warc");

            let config = WarcConfig {
                path: path.to_string_lossy().to_string(),
                write_warcinfo: false,
                software: "test".to_string(),
            };

            let (writer, handle) = WarcWriter::create(&config).unwrap();

            let mut join_handles = Vec::new();
            for t in 0..8u32 {
                let w = writer.clone();
                join_handles.push(tokio::task::spawn(async move {
                    for i in 0..50u32 {
                        let page = make_test_page(
                            &format!("https://t{t}.example.com/{i}"),
                            200,
                            &format!("<html>Thread {t} Page {i}</html>"),
                        );
                        w.write_page(&page);
                    }
                }));
            }

            for h in join_handles {
                h.await.unwrap();
            }

            assert_eq!(writer.record_count(), 400);

            drop(writer);
            handle.await.unwrap().unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            let count = content.matches("WARC/1.1\r\n").count();
            assert_eq!(count, 400);

            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn spawn_warc_writer_processes_pages() {
            let dir = std::env::temp_dir().join("spider_warc_test_spawn");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("test_spawn.warc");

            let config = WarcConfig {
                path: path.to_string_lossy().to_string(),
                write_warcinfo: false,
                software: "test".to_string(),
            };

            let (writer, file_handle) = WarcWriter::create(&config).unwrap();
            let (tx, _) = broadcast::channel::<Page>(16);
            let rx = tx.subscribe();

            let warc_handle = spawn_warc_writer(writer, rx);

            for i in 0..5 {
                let page = make_test_page(
                    &format!("https://example.com/{i}"),
                    200,
                    &format!("Page {i}"),
                );
                tx.send(page).unwrap();
            }

            drop(tx);

            let count = warc_handle.await.unwrap();
            assert_eq!(count, 5);

            // The spawn_warc_writer dropped its writer clone, but we need to
            // ensure file_handle completes too. Wait briefly for channel drain.
            let _ = file_handle.await;

            let content = std::fs::read_to_string(&path).unwrap();
            let record_count = content.matches("WARC/1.1\r\n").count();
            assert_eq!(record_count, 5);

            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn spawn_warc_writer_skips_blocked_pages() {
            let dir = std::env::temp_dir().join("spider_warc_test_blocked");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("test_blocked.warc");

            let config = WarcConfig {
                path: path.to_string_lossy().to_string(),
                write_warcinfo: false,
                software: "test".to_string(),
            };

            let (writer, file_handle) = WarcWriter::create(&config).unwrap();
            let (tx, _) = broadcast::channel::<Page>(16);
            let rx = tx.subscribe();

            let warc_handle = spawn_warc_writer(writer, rx);

            let page = make_test_page("https://example.com/ok", 200, "OK");
            tx.send(page).unwrap();

            let mut blocked = make_test_page("https://example.com/blocked", 200, "Blocked");
            blocked.blocked_crawl = true;
            tx.send(blocked).unwrap();

            drop(tx);

            let count = warc_handle.await.unwrap();
            assert_eq!(count, 1);

            let _ = file_handle.await;

            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn empty_crawl_produces_only_warcinfo() {
            let dir = std::env::temp_dir().join("spider_warc_test_empty_crawl");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("test_empty_crawl.warc");

            let config = WarcConfig {
                path: path.to_string_lossy().to_string(),
                write_warcinfo: true,
                software: "test".to_string(),
            };

            let (writer, handle) = WarcWriter::create(&config).unwrap();
            assert_eq!(writer.record_count(), 1);

            drop(writer);
            handle.await.unwrap().unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            let count = content.matches("WARC/1.1\r\n").count();
            assert_eq!(count, 1);
            assert!(content.contains("WARC-Type: warcinfo"));

            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn response_payload_has_content_type_when_headers_missing() {
            let page = make_test_page("https://example.com", 200, "<html>Hi</html>");
            let buf = serialize_page(&page).unwrap();
            let content = String::from_utf8_lossy(&buf);
            assert!(
                content.contains("Content-Type: text/html; charset=utf-8\r\n"),
                "synthesized Content-Type missing: {content}"
            );
            assert!(content.contains("Content-Length: 15\r\n"));
        }

        #[test]
        fn response_payload_skips_synthesis_when_content_type_present() {
            let mut page = make_test_page("https://example.com", 200, "<html>Hi</html>");
            let mut hdr_map = reqwest::header::HeaderMap::new();
            hdr_map.insert("content-type", "text/plain; charset=utf-8".parse().unwrap());
            page.headers = Some(hdr_map);

            let buf = serialize_page(&page).unwrap();
            let content = String::from_utf8_lossy(&buf);
            // Only the real one should appear (case preserved as sent).
            let ct_count = content
                .matches("content-type: text/plain; charset=utf-8")
                .count();
            assert_eq!(ct_count, 1);
            assert!(!content.contains("Content-Type: text/html"));
        }

        #[test]
        fn guess_content_type_variants() {
            assert_eq!(guess_content_type(b"<html>"), "text/html; charset=utf-8");
            assert_eq!(
                guess_content_type(b"  <!doctype html>"),
                "text/html; charset=utf-8"
            );
            assert_eq!(
                guess_content_type(b"{\"a\":1}"),
                "application/json; charset=utf-8"
            );
            assert_eq!(
                guess_content_type(b"[1,2]"),
                "application/json; charset=utf-8"
            );
            assert_eq!(
                guess_content_type(b"plain text"),
                "text/plain; charset=utf-8"
            );
            assert_eq!(guess_content_type(b""), "application/octet-stream");
            assert_eq!(guess_content_type(b"\x89PNG\r\n\x1a\n"), "image/png");
            assert_eq!(guess_content_type(b"\xff\xd8\xff\xe0"), "image/jpeg");
            assert_eq!(guess_content_type(b"GIF89a"), "image/gif");
            assert_eq!(guess_content_type(b"%PDF-1.4"), "application/pdf");
        }

        #[tokio::test]
        async fn writer_handles_large_pages() {
            let dir = std::env::temp_dir().join("spider_warc_test_large");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("test_large.warc");

            let config = WarcConfig {
                path: path.to_string_lossy().to_string(),
                write_warcinfo: false,
                software: "test".to_string(),
            };

            let (writer, handle) = WarcWriter::create(&config).unwrap();

            // 1 MB page body.
            let large_body = "x".repeat(1_000_000);
            let page = make_test_page("https://example.com/large", 200, &large_body);
            writer.write_page(&page);

            drop(writer);
            handle.await.unwrap().unwrap();

            let bytes = std::fs::read(&path).unwrap();
            // File should be > 1 MB.
            assert!(bytes.len() > 1_000_000);
            assert!(bytes.ends_with(b"\r\n\r\n"));

            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(feature = "warc")]
pub use inner::{serialize_page, spawn_warc_writer, WarcConfig, WarcWriter};
