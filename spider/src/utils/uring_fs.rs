//! Async file I/O with optional io_uring acceleration.
//!
//! On Linux with the `io_uring` feature, file operations are dispatched to a
//! dedicated io_uring worker thread for true kernel-async I/O. On all other
//! platforms (or when io_uring initialization fails), operations transparently
//! fall back to `tokio::fs`.

use std::io;
use tokio::sync::{mpsc, oneshot};

/// Internal operation sent to a streaming writer's background task.
enum StreamOp {
    /// Write a chunk at the current offset.
    Write(Vec<u8>, oneshot::Sender<io::Result<()>>),
    /// Close the file and send the result.
    Close(oneshot::Sender<io::Result<()>>),
}

// ── io_uring implementation ──────────────────────────────────────────────────

#[cfg(all(target_os = "linux", feature = "io_uring"))]
mod inner {
    use std::io;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::{mpsc, oneshot, OnceCell};

    /// Whether the io_uring FS worker is running.
    static URING_FS_ENABLED: AtomicBool = AtomicBool::new(false);

    /// Channel to the io_uring worker thread.
    static URING_FS_POOL: OnceCell<mpsc::UnboundedSender<FileIoTask>> = OnceCell::const_new();

    /// A self-contained file I/O task that can be sent across threads.
    enum FileIoTask {
        WriteFile {
            path: String,
            data: Vec<u8>,
            tx: oneshot::Sender<io::Result<()>>,
        },
        ReadFile {
            path: String,
            tx: oneshot::Sender<io::Result<Vec<u8>>>,
        },
        RemoveFile {
            path: String,
            tx: oneshot::Sender<io::Result<()>>,
        },
        CreateDirAll {
            path: String,
            tx: oneshot::Sender<io::Result<()>>,
        },
        /// Open a file for streaming writes. The io_uring worker spawns a
        /// long-lived sub-task that holds the file handle and processes ops
        /// from the receiver.
        CreateStream {
            path: String,
            ops_rx: mpsc::UnboundedReceiver<super::StreamOp>,
            result_tx: oneshot::Sender<io::Result<()>>,
        },
    }

    /// Initialize the io_uring FS background worker. Returns `true` if
    /// io_uring file I/O is now active.
    pub fn init_uring_fs() -> bool {
        let _ = URING_FS_POOL.set({
            let (tx, mut rx) = mpsc::unbounded_channel::<FileIoTask>();
            let builder = std::thread::Builder::new().name("uring-fs-worker".into());

            if builder
                .spawn(move || {
                    if let Err(e) = tokio_uring::builder().start(async move {
                        while let Some(task) = rx.recv().await {
                            tokio_uring::spawn(dispatch_task(task));
                        }
                    }) {
                        log::error!("io_uring FS worker failed to start: {}", e);
                    }
                })
                .is_err()
            {
                log::warn!("Failed to spawn io_uring FS worker thread");
                let _ = tx.downgrade();
                return;
            }

            URING_FS_ENABLED.store(true, Ordering::Release);
            tx
        });

        URING_FS_ENABLED.load(Ordering::Acquire)
    }

    /// Process a single file I/O task on the io_uring thread.
    async fn dispatch_task(task: FileIoTask) {
        match task {
            FileIoTask::WriteFile { path, data, tx } => {
                let result = async {
                    let file = tokio_uring::fs::File::create(&path).await?;
                    let (res, _) = file.write_all_at(data, 0).await;
                    res?;
                    file.close().await?;
                    Ok(())
                }
                .await;
                let _ = tx.send(result);
            }
            FileIoTask::ReadFile { path, tx } => {
                let result = async {
                    let meta = std::fs::metadata(&path)?;
                    let len = meta.len() as usize;
                    let buf = vec![0u8; len];
                    let file = tokio_uring::fs::File::open(&path).await?;
                    let (res, buf) = file.read_exact_at(buf, 0).await;
                    res?;
                    file.close().await?;
                    Ok(buf)
                }
                .await;
                let _ = tx.send(result);
            }
            FileIoTask::RemoveFile { path, tx } => {
                // No io_uring unlink in v0.5 — use std::fs
                let result = std::fs::remove_file(&path);
                let _ = tx.send(result);
            }
            FileIoTask::CreateDirAll { path, tx } => {
                // mkdir doesn't benefit from io_uring
                let result = std::fs::create_dir_all(&path);
                let _ = tx.send(result);
            }
            FileIoTask::CreateStream {
                path,
                mut ops_rx,
                result_tx,
            } => {
                match tokio_uring::fs::File::create(&path).await {
                    Ok(file) => {
                        let _ = result_tx.send(Ok(()));
                        let mut offset = 0u64;
                        let mut close_tx: Option<oneshot::Sender<io::Result<()>>> = None;

                        while let Some(op) = ops_rx.recv().await {
                            match op {
                                super::StreamOp::Write(data, tx) => {
                                    let len = data.len() as u64;
                                    let (res, _) = file.write_all_at(data, offset).await;
                                    match res {
                                        Ok(()) => {
                                            offset += len;
                                            let _ = tx.send(Ok(()));
                                        }
                                        Err(e) => {
                                            let _ = tx.send(Err(e));
                                        }
                                    }
                                }
                                super::StreamOp::Close(tx) => {
                                    close_tx = Some(tx);
                                    break;
                                }
                            }
                        }

                        // Always close the file (explicit close required for io_uring)
                        let result = file.close().await;
                        if let Some(tx) = close_tx {
                            let _ = tx.send(result);
                        }
                    }
                    Err(e) => {
                        let _ = result_tx.send(Err(e));
                    }
                }
            }
        }
    }

    /// Check if io_uring FS is enabled, and if so, send the task and await the result.
    /// Returns `None` if io_uring is not available (caller should fall back to tokio::fs).
    async fn try_uring<T>(
        make_task: impl FnOnce(oneshot::Sender<io::Result<T>>) -> FileIoTask,
    ) -> Option<io::Result<T>> {
        if !URING_FS_ENABLED.load(Ordering::Acquire) {
            return None;
        }
        let sender = URING_FS_POOL.get()?;
        let (tx, rx) = oneshot::channel();
        if sender.send(make_task(tx)).is_err() {
            return Some(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "io_uring FS worker channel closed",
            )));
        }
        match rx.await {
            Ok(result) => Some(result),
            Err(_) => Some(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "io_uring FS worker dropped the response",
            ))),
        }
    }

    /// Try to create a streaming writer on the io_uring worker thread.
    /// Returns `None` if io_uring is not available.
    pub(super) async fn try_streaming_create(
        path: String,
    ) -> Option<io::Result<mpsc::UnboundedSender<super::StreamOp>>> {
        if !URING_FS_ENABLED.load(Ordering::Acquire) {
            return None;
        }
        let sender = match URING_FS_POOL.get() {
            Some(s) => s,
            None => return None,
        };

        let (ops_tx, ops_rx) = mpsc::unbounded_channel();
        let (result_tx, result_rx) = oneshot::channel();

        if sender
            .send(FileIoTask::CreateStream {
                path,
                ops_rx,
                result_tx,
            })
            .is_err()
        {
            return Some(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "io_uring FS worker channel closed",
            )));
        }

        match result_rx.await {
            Ok(Ok(())) => Some(Ok(ops_tx)),
            Ok(Err(e)) => Some(Err(e)),
            Err(_) => Some(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "io_uring FS worker dropped the response",
            ))),
        }
    }

    /// Write `data` to `path`, creating or truncating the file.
    pub async fn write_file(path: String, data: Vec<u8>) -> io::Result<()> {
        if let Some(result) = try_uring(|tx| FileIoTask::WriteFile {
            path: path.clone(),
            data: data.clone(),
            tx,
        })
        .await
        {
            return result;
        }
        tokio::fs::write(&path, &data).await
    }

    /// Read the entire contents of `path` into a `Vec<u8>`.
    pub async fn read_file(path: String) -> io::Result<Vec<u8>> {
        if let Some(result) = try_uring(|tx| FileIoTask::ReadFile {
            path: path.clone(),
            tx,
        })
        .await
        {
            return result;
        }
        tokio::fs::read(&path).await
    }

    /// Remove a file at `path`.
    pub async fn remove_file(path: String) -> io::Result<()> {
        if let Some(result) = try_uring(|tx| FileIoTask::RemoveFile {
            path: path.clone(),
            tx,
        })
        .await
        {
            return result;
        }
        tokio::fs::remove_file(&path).await
    }

    /// Recursively create directories at `path`.
    pub async fn create_dir_all(path: String) -> io::Result<()> {
        if let Some(result) = try_uring(|tx| FileIoTask::CreateDirAll {
            path: path.clone(),
            tx,
        })
        .await
        {
            return result;
        }
        tokio::fs::create_dir_all(&path).await
    }
}

// ── Fallback implementation (non-Linux or no io_uring feature) ───────────────

#[cfg(not(all(target_os = "linux", feature = "io_uring")))]
mod inner {
    use std::io;
    use tokio::sync::mpsc;

    /// No-op on platforms without io_uring. Always returns `false`.
    pub fn init_uring_fs() -> bool {
        false
    }

    /// No io_uring available — always returns `None`.
    pub(super) async fn try_streaming_create(
        _path: String,
    ) -> Option<io::Result<mpsc::UnboundedSender<super::StreamOp>>> {
        None
    }

    /// Write `data` to `path`, creating or truncating the file.
    pub async fn write_file(path: String, data: Vec<u8>) -> io::Result<()> {
        tokio::fs::write(&path, &data).await
    }

    /// Read the entire contents of `path` into a `Vec<u8>`.
    pub async fn read_file(path: String) -> io::Result<Vec<u8>> {
        tokio::fs::read(&path).await
    }

    /// Remove a file at `path`.
    pub async fn remove_file(path: String) -> io::Result<()> {
        tokio::fs::remove_file(&path).await
    }

    /// Recursively create directories at `path`.
    pub async fn create_dir_all(path: String) -> io::Result<()> {
        tokio::fs::create_dir_all(&path).await
    }
}

// ── Re-exports ───────────────────────────────────────────────────────────────

pub use inner::create_dir_all;
pub use inner::init_uring_fs;
pub use inner::read_file;
pub use inner::remove_file;
pub use inner::write_file;

// ── StreamingWriter ──────────────────────────────────────────────────────────

/// A handle for streaming writes to a file. Writes are dispatched to a
/// background task — on the io_uring worker thread when available, or a
/// spawned tokio task as fallback. The file is created on [`create`] and
/// must be finalized with [`close`].
///
/// If the writer is dropped without calling [`close`], the background task
/// will still close the file (but the caller cannot observe errors).
pub struct StreamingWriter {
    ops_tx: mpsc::UnboundedSender<StreamOp>,
}

impl StreamingWriter {
    /// Create a new file at `path` for streaming writes.
    pub async fn create(path: String) -> io::Result<Self> {
        // Try io_uring path first
        if let Some(result) = inner::try_streaming_create(path.clone()).await {
            return result.map(|ops_tx| Self { ops_tx });
        }
        // Fallback: tokio task
        Self::create_fallback(path).await
    }

    /// Fallback: spawn a tokio task that holds a `tokio::fs::File`.
    async fn create_fallback(path: String) -> io::Result<Self> {
        let file = tokio::fs::File::create(&path).await?;
        let (ops_tx, mut ops_rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut file = file;
            let mut close_tx: Option<oneshot::Sender<io::Result<()>>> = None;

            while let Some(op) = ops_rx.recv().await {
                match op {
                    StreamOp::Write(data, tx) => {
                        let _ = tx.send(file.write_all(&data).await);
                    }
                    StreamOp::Close(tx) => {
                        close_tx = Some(tx);
                        break;
                    }
                }
            }

            let result = file.flush().await;
            if let Some(tx) = close_tx {
                let _ = tx.send(result);
            }
            // file dropped — OS closes the fd
        });

        Ok(Self { ops_tx })
    }

    /// Write a chunk of data at the current offset.
    ///
    /// The data is cloned internally for transfer to the background task.
    /// The caller retains ownership of the source buffer.
    pub async fn write(&self, data: &[u8]) -> io::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.ops_tx
            .send(StreamOp::Write(data.to_vec(), tx))
            .map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "streaming writer task exited")
            })?;
        rx.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "streaming writer dropped the response",
            )
        })?
    }

    /// Close the file and wait for completion.
    pub async fn close(self) -> io::Result<()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.ops_tx.send(StreamOp::Close(tx));
        rx.await.unwrap_or(Ok(()))
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> String {
        let dir = std::env::temp_dir();
        dir.join(format!("spider_uring_fs_test_{}", name))
            .display()
            .to_string()
    }

    #[tokio::test]
    async fn test_write_read_remove_fallback() {
        let path = temp_path("fallback");
        let payload = b"hello uring_fs fallback".to_vec();

        write_file(path.clone(), payload.clone()).await.unwrap();

        let read_back = read_file(path.clone()).await.unwrap();
        assert_eq!(read_back, payload);

        remove_file(path.clone()).await.unwrap();
        assert!(read_file(path).await.is_err());
    }

    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    #[tokio::test]
    async fn test_write_read_remove_uring() {
        let _ = init_uring_fs();
        let path = temp_path("uring");
        let payload = vec![0xABu8; 4096]; // 4 KB

        write_file(path.clone(), payload.clone()).await.unwrap();

        let read_back = read_file(path.clone()).await.unwrap();
        assert_eq!(read_back, payload);

        remove_file(path.clone()).await.unwrap();
        assert!(read_file(path).await.is_err());
    }

    #[tokio::test]
    async fn test_fallback_when_not_initialized() {
        // Without calling init_uring_fs(), should still work via tokio::fs
        let path = temp_path("no_init");
        let payload = b"no init test".to_vec();

        write_file(path.clone(), payload.clone()).await.unwrap();
        let read_back = read_file(path.clone()).await.unwrap();
        assert_eq!(read_back, payload);
        let _ = remove_file(path).await;
    }

    #[tokio::test]
    async fn test_write_empty_file() {
        let path = temp_path("empty");

        write_file(path.clone(), Vec::new()).await.unwrap();

        let read_back = read_file(path.clone()).await.unwrap();
        assert!(read_back.is_empty());

        let _ = remove_file(path).await;
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let path = temp_path("nonexistent_surely");
        let result = read_file(path).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_streaming_writer_fallback() {
        let path = temp_path("streaming_fallback");

        let writer = StreamingWriter::create(path.clone()).await.unwrap();
        writer.write(b"chunk1").await.unwrap();
        writer.write(b"chunk2").await.unwrap();
        writer.write(b"chunk3").await.unwrap();
        writer.close().await.unwrap();

        let content = read_file(path.clone()).await.unwrap();
        assert_eq!(content, b"chunk1chunk2chunk3");

        let _ = remove_file(path).await;
    }

    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    #[tokio::test]
    async fn test_streaming_writer_uring() {
        let _ = init_uring_fs();
        let path = temp_path("streaming_uring");

        let writer = StreamingWriter::create(path.clone()).await.unwrap();
        // Write a larger payload in multiple chunks
        let chunk = vec![0xCDu8; 4096];
        for _ in 0..4 {
            writer.write(&chunk).await.unwrap();
        }
        writer.close().await.unwrap();

        let content = read_file(path.clone()).await.unwrap();
        assert_eq!(content.len(), 4096 * 4);
        assert!(content.iter().all(|&b| b == 0xCD));

        let _ = remove_file(path).await;
    }

    #[tokio::test]
    async fn test_streaming_writer_drop_without_close() {
        let path = temp_path("streaming_drop");

        let writer = StreamingWriter::create(path.clone()).await.unwrap();
        writer.write(b"data before drop").await.unwrap();
        drop(writer);

        // Give the background task a moment to close
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let content = read_file(path.clone()).await.unwrap();
        assert_eq!(content, b"data before drop");

        let _ = remove_file(path).await;
    }
}
