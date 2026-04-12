//! Async file I/O with optional io_uring acceleration.
//!
//! On Linux with the `io_uring` feature, file operations are dispatched to a
//! dedicated worker thread that drives a raw io_uring ring for true
//! kernel-async I/O. On all other platforms (or when io_uring is unavailable
//! at runtime — e.g. seccomp-filtered containers, older kernels), operations
//! transparently fall back to `tokio::fs`.
//!
//! **No mutexes. No async runtime on the worker.** The worker thread runs a
//! tight synchronous loop: `blocking_recv` → io_uring submit → reap CQE →
//! send result back via oneshot.

use std::io;
use tokio::sync::{mpsc, oneshot};

/// Internal operation sent to a streaming writer's background task.
enum StreamOp {
    /// Write a chunk at the current offset.
    Write(Vec<u8>, oneshot::Sender<io::Result<()>>),
    /// Close the file and send the result.
    Close(oneshot::Sender<io::Result<()>>),
}

// ── io_uring implementation (raw io-uring crate) ────────────────────────────

#[cfg(all(target_os = "linux", feature = "io_uring"))]
mod inner {
    use std::ffi::CString;
    use std::io;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::{mpsc, oneshot};

    /// Whether the io_uring FS worker is running and healthy.
    static URING_FS_ENABLED: AtomicBool = AtomicBool::new(false);

    /// Channel to the io_uring worker thread.
    /// `mpsc::UnboundedSender` is `Send + Sync`, safe for `OnceLock`.
    static URING_FS_POOL: std::sync::OnceLock<mpsc::UnboundedSender<FileIoTask>> =
        std::sync::OnceLock::new();

    /// A self-contained I/O task sent to the worker thread.
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
        /// TCP connect via io_uring Socket + Connect opcodes.
        TcpConnect {
            addr: std::net::SocketAddr,
            tx: oneshot::Sender<io::Result<std::net::TcpStream>>,
        },
        /// Send data on a raw fd via io_uring Send opcode.
        TcpSend {
            fd: i32,
            data: Vec<u8>,
            tx: oneshot::Sender<io::Result<usize>>,
        },
        /// Receive data from a raw fd via io_uring Recv opcode.
        TcpRecv {
            fd: i32,
            buf_len: u32,
            tx: oneshot::Sender<io::Result<Vec<u8>>>,
        },
        /// Splice data between two fds via a kernel pipe (zero-copy).
        #[cfg(feature = "splice")]
        Splice {
            fd_in: i32,
            fd_out: i32,
            len: usize,
            tx: oneshot::Sender<io::Result<usize>>,
        },
    }

    // ── Kernel probe ────────────────────────────────────────────────────────

    /// Try to create a minimal io_uring ring. If the kernel or seccomp policy
    /// blocks it (ENOSYS, EPERM, etc.), returns `None`.
    pub(crate) fn probe_io_uring() -> Option<io_uring::IoUring> {
        match io_uring::IoUring::builder().build(64) {
            Ok(ring) => {
                log::info!("io_uring probe succeeded — kernel supports io_uring");
                Some(ring)
            }
            Err(e) => {
                log::info!(
                    "io_uring unavailable ({}), file I/O will use tokio::fs fallback",
                    e
                );
                None
            }
        }
    }

    // ── Worker loop (synchronous, no async runtime) ─────────────────────────

    /// The worker thread's main loop. Receives tasks via blocking channel recv,
    /// processes each through the io_uring ring, sends results back via oneshot.
    /// Exits cleanly when the channel sender is dropped.
    fn worker_loop(mut rx: mpsc::UnboundedReceiver<FileIoTask>, mut ring: io_uring::IoUring) {
        while let Some(task) = rx.blocking_recv() {
            match task {
                FileIoTask::WriteFile { path, data, tx } => {
                    let _ = tx.send(uring_write_file(&mut ring, &path, &data));
                }
                FileIoTask::ReadFile { path, tx } => {
                    let _ = tx.send(uring_read_file(&mut ring, &path));
                }
                FileIoTask::RemoveFile { path, tx } => {
                    // unlinkat not widely supported in io_uring 0.7 — use std
                    let _ = tx.send(std::fs::remove_file(&path));
                }
                FileIoTask::CreateDirAll { path, tx } => {
                    // mkdir doesn't benefit from io_uring
                    let _ = tx.send(std::fs::create_dir_all(&path));
                }
                FileIoTask::TcpConnect { addr, tx } => {
                    let _ = tx.send(uring_tcp_connect(&mut ring, addr));
                }
                FileIoTask::TcpSend { fd, data, tx } => {
                    let _ = tx.send(uring_tcp_send(&mut ring, fd, &data));
                }
                FileIoTask::TcpRecv { fd, buf_len, tx } => {
                    let _ = tx.send(uring_tcp_recv(&mut ring, fd, buf_len));
                }
                #[cfg(feature = "splice")]
                FileIoTask::Splice {
                    fd_in,
                    fd_out,
                    len,
                    tx,
                } => {
                    let _ = tx.send(uring_splice(&mut ring, fd_in, fd_out, len));
                }
            }
        }
        // Channel closed — drop ring (closes the io_uring fd)
        drop(ring);
    }

    // ── io_uring file operations ────────────────────────────────────────────

    /// Submit one SQE, wait for one CQE, return the result code.
    fn submit_and_reap(ring: &mut io_uring::IoUring) -> io::Result<i32> {
        ring.submit_and_wait(1)?;
        let cqe = ring
            .completion()
            .next()
            .ok_or_else(|| io::Error::other("io_uring: no CQE after wait"))?;
        Ok(cqe.result())
    }

    /// Close an fd through io_uring. Best-effort — errors are returned but
    /// the fd is consumed regardless.
    fn uring_close(ring: &mut io_uring::IoUring, fd: i32) -> io::Result<()> {
        let close_e = io_uring::opcode::Close::new(io_uring::types::Fd(fd))
            .build()
            .user_data(0xC105E);

        // SAFETY: SQE references only the fd integer, no buffer pointers.
        unsafe {
            ring.submission()
                .push(&close_e)
                .map_err(|_| io::Error::other("io_uring: SQ full on close"))?;
        }

        let res = submit_and_reap(ring)?;
        if res < 0 {
            return Err(io::Error::from_raw_os_error(-res));
        }
        Ok(())
    }

    /// Write `data` to `path` (create/truncate) using io_uring OpenAt → Write
    /// loop → Close.
    fn uring_write_file(ring: &mut io_uring::IoUring, path: &str, data: &[u8]) -> io::Result<()> {
        let c_path =
            CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        // ── Open (O_WRONLY | O_CREAT | O_TRUNC) ────────────────────────────
        let open_e =
            io_uring::opcode::OpenAt::new(io_uring::types::Fd(libc::AT_FDCWD), c_path.as_ptr())
                .flags(libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC)
                .mode(0o644)
                .build()
                .user_data(0x0BE4);

        // SAFETY: c_path is alive and pinned on the stack until after
        // submit_and_reap returns.
        unsafe {
            ring.submission()
                .push(&open_e)
                .map_err(|_| io::Error::other("io_uring: SQ full on open"))?;
        }

        let fd = submit_and_reap(ring)?;
        if fd < 0 {
            return Err(io::Error::from_raw_os_error(-fd));
        }

        // ── Write (loop for short writes) ───────────────────────────────────
        let write_result = uring_write_all(ring, fd, data);

        // ── Close (always, even on write error) ─────────────────────────────
        let close_result = uring_close(ring, fd);

        // Return first error encountered.
        write_result?;
        close_result
    }

    /// Write all of `data` to `fd` at offset 0, handling short writes.
    fn uring_write_all(ring: &mut io_uring::IoUring, fd: i32, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        let mut offset: u64 = 0;
        while (offset as usize) < data.len() {
            let remaining = &data[offset as usize..];
            // io_uring Write len is u32 — cap each submission.
            let chunk_len = remaining.len().min(u32::MAX as usize) as u32;

            let write_e = io_uring::opcode::Write::new(
                io_uring::types::Fd(fd),
                remaining.as_ptr(),
                chunk_len,
            )
            .offset(offset)
            .build()
            .user_data(0x1417E);

            // SAFETY: `remaining` (a slice of `data`) is alive on the stack
            // and won't move before submit_and_reap returns.
            unsafe {
                ring.submission()
                    .push(&write_e)
                    .map_err(|_| io::Error::other("io_uring: SQ full on write"))?;
            }

            let written = submit_and_reap(ring)?;
            if written < 0 {
                return Err(io::Error::from_raw_os_error(-written));
            }
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "io_uring: write returned 0 bytes",
                ));
            }
            offset += written as u64;
        }
        Ok(())
    }

    /// Read the entire file at `path` using io_uring OpenAt → Read loop → Close.
    fn uring_read_file(ring: &mut io_uring::IoUring, path: &str) -> io::Result<Vec<u8>> {
        // Get file size via std::fs (fast, synchronous stat).
        let meta = std::fs::metadata(path)?;
        let len = meta.len() as usize;

        if len == 0 {
            return Ok(Vec::new());
        }

        let c_path =
            CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        // ── Open (O_RDONLY) ─────────────────────────────────────────────────
        let open_e =
            io_uring::opcode::OpenAt::new(io_uring::types::Fd(libc::AT_FDCWD), c_path.as_ptr())
                .flags(libc::O_RDONLY)
                .build()
                .user_data(0x0BE4);

        // SAFETY: c_path alive until after submit_and_reap.
        unsafe {
            ring.submission()
                .push(&open_e)
                .map_err(|_| io::Error::other("io_uring: SQ full on open"))?;
        }

        let fd = submit_and_reap(ring)?;
        if fd < 0 {
            return Err(io::Error::from_raw_os_error(-fd));
        }

        // ── Read (loop for short reads) ─────────────────────────────────────
        let mut buf = vec![0u8; len];
        let read_result = uring_read_exact(ring, fd, &mut buf);

        // ── Close ───────────────────────────────────────────────────────────
        let close_result = uring_close(ring, fd);

        read_result?;
        close_result?;
        Ok(buf)
    }

    /// Read exactly `buf.len()` bytes from `fd` at offset 0, handling short reads.
    fn uring_read_exact(ring: &mut io_uring::IoUring, fd: i32, buf: &mut [u8]) -> io::Result<()> {
        let mut offset: u64 = 0;
        while (offset as usize) < buf.len() {
            let remaining = &mut buf[offset as usize..];
            let chunk_len = remaining.len().min(u32::MAX as usize) as u32;

            let read_e = io_uring::opcode::Read::new(
                io_uring::types::Fd(fd),
                remaining.as_mut_ptr(),
                chunk_len,
            )
            .offset(offset)
            .build()
            .user_data(0x4EAD);

            // SAFETY: `remaining` is a mutable slice of `buf` on the heap.
            // It won't move before submit_and_reap returns.
            unsafe {
                ring.submission()
                    .push(&read_e)
                    .map_err(|_| io::Error::other("io_uring: SQ full on read"))?;
            }

            let n = submit_and_reap(ring)?;
            if n < 0 {
                return Err(io::Error::from_raw_os_error(-n));
            }
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "io_uring: read returned 0 (unexpected EOF)",
                ));
            }
            offset += n as u64;
        }
        Ok(())
    }

    // ── io_uring TCP connect ───────────────────────────────────────────────

    /// Create a TCP socket and connect to `addr` using io_uring
    /// Socket + Connect opcodes. Returns an owned `TcpStream`.
    fn uring_tcp_connect(
        ring: &mut io_uring::IoUring,
        addr: std::net::SocketAddr,
    ) -> io::Result<std::net::TcpStream> {
        use std::os::unix::io::FromRawFd;

        let domain = match addr {
            std::net::SocketAddr::V4(_) => libc::AF_INET,
            std::net::SocketAddr::V6(_) => libc::AF_INET6,
        };

        // ── Socket (SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC) ─────────
        let socket_e = io_uring::opcode::Socket::new(
            domain,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
        .build()
        .user_data(0x50CE7);

        // SAFETY: no buffer pointers, just integer parameters.
        unsafe {
            ring.submission()
                .push(&socket_e)
                .map_err(|_| io::Error::other("io_uring: SQ full on socket"))?;
        }

        let fd = submit_and_reap(ring)?;
        if fd < 0 {
            return Err(io::Error::from_raw_os_error(-fd));
        }

        // ── TCP Fast Open ────────────────────────────────────────────────────
        // Set TCP_FASTOPEN_CONNECT before connect — saves 1 RTT for repeat hosts.
        #[cfg(feature = "tcp_fastopen")]
        {
            const TCP_FASTOPEN_CONNECT: libc::c_int = 30;
            let enable: libc::c_int = 1;
            // Best-effort — if the kernel doesn't support it, we just proceed normally.
            unsafe {
                libc::setsockopt(
                    fd,
                    libc::IPPROTO_TCP,
                    TCP_FASTOPEN_CONNECT,
                    &enable as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        // ── Connect ─────────────────────────────────────────────────────────
        // Build sockaddr on the stack — must live until submit_and_reap returns.
        let (sa_ptr, sa_len) = match addr {
            std::net::SocketAddr::V4(v4) => {
                let sa = libc::sockaddr_in {
                    sin_family: libc::AF_INET as libc::sa_family_t,
                    sin_port: v4.port().to_be(),
                    sin_addr: libc::in_addr {
                        s_addr: u32::from_ne_bytes(v4.ip().octets()),
                    },
                    sin_zero: [0; 8],
                };
                // SAFETY: sa lives on the stack until after submit_and_reap.
                let ptr = &sa as *const libc::sockaddr_in as *const libc::sockaddr;
                (ptr, std::mem::size_of::<libc::sockaddr_in>() as u32)
            }
            std::net::SocketAddr::V6(v6) => {
                let sa = libc::sockaddr_in6 {
                    sin6_family: libc::AF_INET6 as libc::sa_family_t,
                    sin6_port: v6.port().to_be(),
                    sin6_flowinfo: v6.flowinfo(),
                    sin6_addr: libc::in6_addr {
                        s6_addr: v6.ip().octets(),
                    },
                    sin6_scope_id: v6.scope_id(),
                };
                let ptr = &sa as *const libc::sockaddr_in6 as *const libc::sockaddr;
                (ptr, std::mem::size_of::<libc::sockaddr_in6>() as u32)
            }
        };

        let connect_e = io_uring::opcode::Connect::new(io_uring::types::Fd(fd), sa_ptr, sa_len)
            .build()
            .user_data(0xC044);

        // SAFETY: sockaddr struct on the stack is alive until submit_and_reap returns.
        unsafe {
            ring.submission().push(&connect_e).map_err(|_| {
                // Close the socket on error.
                libc::close(fd);
                io::Error::other("io_uring: SQ full on connect")
            })?;
        }

        let res = submit_and_reap(ring)?;

        // EINPROGRESS is normal for non-blocking sockets — io_uring resolves
        // it before returning the CQE, so res==0 means connected.
        // Any other negative value is an error.
        if res < 0 && res != -libc::EINPROGRESS {
            // Close the fd we created.
            let _ = uring_close(ring, fd);
            return Err(io::Error::from_raw_os_error(-res));
        }

        // SAFETY: we own this fd, it's a valid connected TCP socket.
        let stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
        Ok(stream)
    }

    // ── io_uring TCP send/recv ────────────────────────────────────────────

    /// Send `data` on a connected socket fd via io_uring Send opcode.
    /// Returns the number of bytes sent.
    fn uring_tcp_send(ring: &mut io_uring::IoUring, fd: i32, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }

        let mut total: usize = 0;
        while total < data.len() {
            let remaining = &data[total..];
            let chunk_len = remaining.len().min(u32::MAX as usize) as u32;

            let send_e =
                io_uring::opcode::Send::new(io_uring::types::Fd(fd), remaining.as_ptr(), chunk_len)
                    .build()
                    .user_data(0x5E4D);

            // SAFETY: `remaining` is a slice of `data` alive until submit_and_reap.
            unsafe {
                ring.submission()
                    .push(&send_e)
                    .map_err(|_| io::Error::other("io_uring: SQ full on send"))?;
            }

            let n = submit_and_reap(ring)?;
            if n < 0 {
                return Err(io::Error::from_raw_os_error(-n));
            }
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "io_uring: send returned 0 bytes",
                ));
            }
            total += n as usize;
        }
        Ok(total)
    }

    /// Receive up to `buf_len` bytes from a connected socket fd via io_uring
    /// Recv opcode. Returns the received data.
    fn uring_tcp_recv(ring: &mut io_uring::IoUring, fd: i32, buf_len: u32) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; buf_len as usize];

        let recv_e =
            io_uring::opcode::Recv::new(io_uring::types::Fd(fd), buf.as_mut_ptr(), buf_len)
                .build()
                .user_data(0x4EC7);

        // SAFETY: `buf` is heap-allocated and alive until submit_and_reap returns.
        unsafe {
            ring.submission()
                .push(&recv_e)
                .map_err(|_| io::Error::other("io_uring: SQ full on recv"))?;
        }

        let n = submit_and_reap(ring)?;
        if n < 0 {
            return Err(io::Error::from_raw_os_error(-n));
        }

        buf.truncate(n as usize);
        Ok(buf)
    }

    // ── io_uring splice (zero-copy fd-to-fd via kernel pipe) ─────────────

    /// Splice `len` bytes from `fd_in` to `fd_out` via a kernel pipe.
    /// Both fds must support splice (sockets, files, pipes).
    /// Returns total bytes spliced.
    #[cfg(feature = "splice")]
    fn uring_splice(
        ring: &mut io_uring::IoUring,
        fd_in: i32,
        fd_out: i32,
        len: usize,
    ) -> io::Result<usize> {
        // Create a kernel pipe for the splice bridge.
        let mut pipe_fds = [0i32; 2];
        let ret = unsafe { libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_CLOEXEC) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        let pipe_read = pipe_fds[0];
        let pipe_write = pipe_fds[1];

        let mut total: usize = 0;
        let result = (|| -> io::Result<usize> {
            while total < len {
                let remaining = (len - total).min(u32::MAX as usize) as u32;

                // Phase 1: splice fd_in → pipe_write
                let splice_in = io_uring::opcode::Splice::new(
                    io_uring::types::Fd(fd_in),
                    -1i64,
                    io_uring::types::Fd(pipe_write),
                    -1i64,
                    remaining,
                )
                .flags(libc::SPLICE_F_MOVE | libc::SPLICE_F_NONBLOCK)
                .build()
                .user_data(0x5B1C_0E14);

                unsafe {
                    ring.submission()
                        .push(&splice_in)
                        .map_err(|_| io::Error::other("io_uring: SQ full on splice-in"))?;
                }

                let n_in = submit_and_reap(ring)?;
                if n_in < 0 {
                    return Err(io::Error::from_raw_os_error(-n_in));
                }
                if n_in == 0 {
                    break; // EOF on input
                }

                // Phase 2: splice pipe_read → fd_out
                let splice_out = io_uring::opcode::Splice::new(
                    io_uring::types::Fd(pipe_read),
                    -1i64,
                    io_uring::types::Fd(fd_out),
                    -1i64,
                    n_in as u32,
                )
                .flags(libc::SPLICE_F_MOVE)
                .build()
                .user_data(0x5B1C_0E07);

                unsafe {
                    ring.submission()
                        .push(&splice_out)
                        .map_err(|_| io::Error::other("io_uring: SQ full on splice-out"))?;
                }

                let n_out = submit_and_reap(ring)?;
                if n_out < 0 {
                    return Err(io::Error::from_raw_os_error(-n_out));
                }
                total += n_out as usize;
            }
            Ok(total)
        })();

        // Always close the pipe fds.
        unsafe {
            libc::close(pipe_read);
            libc::close(pipe_write);
        }

        result
    }

    // ── Public API ──────────────────────────────────────────────────────────

    /// Probe the kernel for io_uring support and, if available, spawn the
    /// worker thread. Returns `true` when io_uring file I/O is active.
    ///
    /// Safe to call multiple times — only the first successful call has
    /// effect. Subsequent calls return the cached status.
    pub fn init_uring_fs() -> bool {
        // Fast path: already initialized.
        if URING_FS_ENABLED.load(Ordering::Acquire) {
            return true;
        }

        // Probe: try to create a ring. Fails gracefully on AWS AL2, ECS,
        // Lambda, seccomp-filtered containers, kernels < 5.1, etc.
        let ring = match probe_io_uring() {
            Some(r) => r,
            None => return false,
        };

        let (tx, rx) = mpsc::unbounded_channel();
        let builder = std::thread::Builder::new().name("uring-fs-worker".into());

        match builder.spawn(move || worker_loop(rx, ring)) {
            Ok(_) => {
                if URING_FS_POOL.set(tx).is_ok() {
                    URING_FS_ENABLED.store(true, Ordering::Release);
                }
                // If set() failed, another thread won the race. Our worker
                // will exit when its rx is dropped — no leak.
            }
            Err(e) => {
                log::warn!("Failed to spawn io_uring FS worker thread: {}", e);
                return false;
            }
        }

        URING_FS_ENABLED.load(Ordering::Acquire)
    }

    /// Send a task to the io_uring worker and await the result.
    /// Returns `None` if io_uring is not available (caller falls back).
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

    /// Streaming writes always use the tokio::fs fallback.
    /// io_uring doesn't add value for sequential single-writer streaming.
    pub(super) async fn try_streaming_create(
        _path: String,
    ) -> Option<io::Result<mpsc::UnboundedSender<super::StreamOp>>> {
        None
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

    /// TCP connect via io_uring. Returns a connected `std::net::TcpStream`.
    /// Falls back to blocking `TcpStream::connect` if io_uring is unavailable.
    pub async fn tcp_connect(addr: std::net::SocketAddr) -> io::Result<std::net::TcpStream> {
        if let Some(result) = try_uring(|tx| FileIoTask::TcpConnect { addr, tx }).await {
            return result;
        }
        // Fallback: blocking connect on a spawn_blocking thread.
        tokio::task::spawn_blocking(move || std::net::TcpStream::connect(addr))
            .await
            .map_err(io::Error::other)?
    }

    /// Send data on a connected socket fd via io_uring.
    /// Falls back to blocking `libc::send` if io_uring is unavailable.
    pub async fn tcp_send(fd: i32, data: Vec<u8>) -> io::Result<usize> {
        if let Some(result) = try_uring(|tx| FileIoTask::TcpSend {
            fd,
            data: data.clone(),
            tx,
        })
        .await
        {
            return result;
        }
        // Fallback: blocking send.
        let data_clone = data;
        tokio::task::spawn_blocking(move || {
            let n = unsafe {
                libc::send(
                    fd,
                    data_clone.as_ptr() as *const libc::c_void,
                    data_clone.len(),
                    0,
                )
            };
            if n < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Receive up to `buf_len` bytes from a connected socket fd via io_uring.
    /// Falls back to blocking `libc::recv` if io_uring is unavailable.
    pub async fn tcp_recv(fd: i32, buf_len: u32) -> io::Result<Vec<u8>> {
        if let Some(result) = try_uring(|tx| FileIoTask::TcpRecv { fd, buf_len, tx }).await {
            return result;
        }
        // Fallback: blocking recv.
        tokio::task::spawn_blocking(move || {
            let mut buf = vec![0u8; buf_len as usize];
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
            if n < 0 {
                Err(io::Error::last_os_error())
            } else {
                buf.truncate(n as usize);
                Ok(buf)
            }
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Splice `len` bytes from `fd_in` to `fd_out` via a kernel pipe (zero-copy).
    /// Falls back to userspace read+write if io_uring is unavailable.
    #[cfg(feature = "splice")]
    pub async fn splice(fd_in: i32, fd_out: i32, len: usize) -> io::Result<usize> {
        if let Some(result) = try_uring(|tx| FileIoTask::Splice {
            fd_in,
            fd_out,
            len,
            tx,
        })
        .await
        {
            return result;
        }
        // Fallback: libc::splice (still zero-copy, just synchronous).
        tokio::task::spawn_blocking(move || {
            let mut pipe_fds = [0i32; 2];
            let ret = unsafe { libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_CLOEXEC) };
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }
            let pipe_read = pipe_fds[0];
            let pipe_write = pipe_fds[1];

            let mut total: usize = 0;
            let result = (|| -> io::Result<usize> {
                while total < len {
                    let remaining = (len - total).min(65536);
                    let n_in = unsafe {
                        libc::splice(
                            fd_in,
                            std::ptr::null_mut(),
                            pipe_write,
                            std::ptr::null_mut(),
                            remaining,
                            libc::SPLICE_F_MOVE | libc::SPLICE_F_NONBLOCK,
                        )
                    };
                    if n_in < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if n_in == 0 {
                        break;
                    }
                    let n_out = unsafe {
                        libc::splice(
                            pipe_read,
                            std::ptr::null_mut(),
                            fd_out,
                            std::ptr::null_mut(),
                            n_in as usize,
                            libc::SPLICE_F_MOVE,
                        )
                    };
                    if n_out < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    total += n_out as usize;
                }
                Ok(total)
            })();

            unsafe {
                libc::close(pipe_read);
                libc::close(pipe_write);
            }

            result
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Whether io_uring is currently active.
    pub fn is_uring_enabled() -> bool {
        URING_FS_ENABLED.load(Ordering::Acquire)
    }

    /// Read a file and feed it chunk-by-chunk to a callback.
    ///
    /// **Optimal strategy per platform:**
    /// - io_uring available: single kernel-async read, chunk from memory
    ///   (1 syscall, zero spawn_blocking).
    /// - Fallback: stream via `tokio::fs::File` + `BufReader` so the full
    ///   file is never in memory (bounded by `chunk_size`).
    ///
    /// Returns total bytes read.  The callback can return `false` to stop
    /// early (e.g. on a parse error).
    pub async fn read_file_chunked(
        path: String,
        chunk_size: usize,
        mut cb: impl FnMut(&[u8]) -> bool,
    ) -> io::Result<usize> {
        let chunk_size = chunk_size.max(1);

        // Fast path: io_uring single read → chunk from memory.
        if let Some(result) = try_uring(|tx| FileIoTask::ReadFile {
            path: path.clone(),
            tx,
        })
        .await
        {
            let data = result?;
            let mut total = 0usize;
            for (i, chunk) in data.chunks(chunk_size).enumerate() {
                total += chunk.len();
                if !cb(chunk) {
                    break;
                }
                // Yield every 8 chunks (~512 KiB at 64 KiB chunks) so
                // CPU-heavy consumers don't starve the executor.
                if i & 7 == 7 {
                    tokio::task::yield_now().await;
                }
            }
            return Ok(total);
        }

        // Fallback: stream via tokio::fs, bounded memory.
        use tokio::io::AsyncReadExt;
        let file = tokio::fs::File::open(&path).await?;
        let mut reader = tokio::io::BufReader::with_capacity(chunk_size, file);
        let mut buf = vec![0u8; chunk_size];
        let mut total = 0usize;
        loop {
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            total += n;
            if !cb(&buf[..n]) {
                break;
            }
        }
        Ok(total)
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

    /// TCP connect — no io_uring, uses blocking fallback.
    pub async fn tcp_connect(addr: std::net::SocketAddr) -> io::Result<std::net::TcpStream> {
        tokio::task::spawn_blocking(move || std::net::TcpStream::connect(addr))
            .await
            .map_err(io::Error::other)?
    }

    /// Send data on a socket fd — no io_uring fallback.
    #[cfg(target_os = "linux")]
    pub async fn tcp_send(fd: i32, data: Vec<u8>) -> io::Result<usize> {
        tokio::task::spawn_blocking(move || {
            let n = unsafe { libc::send(fd, data.as_ptr() as *const libc::c_void, data.len(), 0) };
            if n < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Receive data from a socket fd — no io_uring fallback.
    #[cfg(target_os = "linux")]
    pub async fn tcp_recv(fd: i32, buf_len: u32) -> io::Result<Vec<u8>> {
        tokio::task::spawn_blocking(move || {
            let mut buf = vec![0u8; buf_len as usize];
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
            if n < 0 {
                Err(io::Error::last_os_error())
            } else {
                buf.truncate(n as usize);
                Ok(buf)
            }
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Splice data between fds — no io_uring, uses libc::splice.
    #[cfg(all(target_os = "linux", feature = "splice"))]
    pub async fn splice(fd_in: i32, fd_out: i32, len: usize) -> io::Result<usize> {
        tokio::task::spawn_blocking(move || {
            let mut pipe_fds = [0i32; 2];
            let ret = unsafe { libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_CLOEXEC) };
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }
            let pipe_read = pipe_fds[0];
            let pipe_write = pipe_fds[1];

            let mut total: usize = 0;
            let result = (|| -> io::Result<usize> {
                while total < len {
                    let remaining = (len - total).min(65536);
                    let n_in = unsafe {
                        libc::splice(
                            fd_in,
                            std::ptr::null_mut(),
                            pipe_write,
                            std::ptr::null_mut(),
                            remaining,
                            libc::SPLICE_F_MOVE | libc::SPLICE_F_NONBLOCK,
                        )
                    };
                    if n_in < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if n_in == 0 {
                        break;
                    }
                    let n_out = unsafe {
                        libc::splice(
                            pipe_read,
                            std::ptr::null_mut(),
                            fd_out,
                            std::ptr::null_mut(),
                            n_in as usize,
                            libc::SPLICE_F_MOVE,
                        )
                    };
                    if n_out < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    total += n_out as usize;
                }
                Ok(total)
            })();

            unsafe {
                libc::close(pipe_read);
                libc::close(pipe_write);
            }

            result
        })
        .await
        .map_err(io::Error::other)?
    }

    /// io_uring is never enabled on this platform.
    pub fn is_uring_enabled() -> bool {
        false
    }

    /// Read a file and feed it chunk-by-chunk to a callback.
    /// Non-uring fallback: streams via `tokio::fs::File` + `BufReader`.
    pub async fn read_file_chunked(
        path: String,
        chunk_size: usize,
        mut cb: impl FnMut(&[u8]) -> bool,
    ) -> io::Result<usize> {
        use tokio::io::AsyncReadExt;
        let chunk_size = chunk_size.max(1);
        let file = tokio::fs::File::open(&path).await?;
        let mut reader = tokio::io::BufReader::with_capacity(chunk_size, file);
        let mut buf = vec![0u8; chunk_size];
        let mut total = 0usize;
        loop {
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            total += n;
            if !cb(&buf[..n]) {
                break;
            }
        }
        Ok(total)
    }
}

// ── Re-exports ───────────────────────────────────────────────────────────────

pub use inner::create_dir_all;
pub use inner::init_uring_fs;
pub use inner::is_uring_enabled;
pub use inner::read_file;
pub use inner::read_file_chunked;
pub use inner::remove_file;
pub use inner::tcp_connect;
pub use inner::write_file;

// Linux-only re-exports for network I/O.
#[cfg(all(target_os = "linux", feature = "splice"))]
pub use inner::splice;
#[cfg(target_os = "linux")]
pub use inner::tcp_recv;
#[cfg(target_os = "linux")]
pub use inner::tcp_send;

// ── StreamingWriter ──────────────────────────────────────────────────────────

/// A handle for streaming writes to a file. Writes are dispatched to a
/// background task — always via a spawned tokio task (io_uring doesn't help
/// sequential single-writer streaming). The file is created on [`create`]
/// and must be finalized with [`close`].
///
/// If the writer is dropped without calling [`close`], the background task
/// will still close the file (but the caller cannot observe errors).
pub struct StreamingWriter {
    ops_tx: mpsc::UnboundedSender<StreamOp>,
}

impl StreamingWriter {
    /// Create a new file at `path` for streaming writes.
    pub async fn create(path: String) -> io::Result<Self> {
        // Try io_uring path first (currently always returns None —
        // streaming doesn't benefit from io_uring).
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

    // ── Fallback path tests (run on all platforms) ───────────────────────

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

    #[tokio::test]
    async fn test_create_dir_all_fallback() {
        let base = temp_path("dir_test");
        let nested = format!("{}/a/b/c", base);

        create_dir_all(nested.clone()).await.unwrap();
        assert!(std::path::Path::new(&nested).is_dir());

        // Cleanup
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn test_init_idempotent() {
        // Calling init multiple times must not panic or deadlock.
        let r1 = init_uring_fs();
        let r2 = init_uring_fs();
        // Both should return the same value (platform-dependent).
        assert_eq!(r1, r2);
    }

    #[tokio::test]
    async fn test_large_write_read() {
        let path = temp_path("large_file");
        // 1 MB payload — tests short-write loop handling.
        let payload = vec![0xABu8; 1024 * 1024];

        write_file(path.clone(), payload.clone()).await.unwrap();

        let read_back = read_file(path.clone()).await.unwrap();
        assert_eq!(read_back.len(), payload.len());
        assert_eq!(read_back, payload);

        let _ = remove_file(path).await;
    }

    #[tokio::test]
    async fn test_concurrent_writes() {
        // Spawn 8 concurrent write+read tasks. No deadlocks, no data
        // corruption.
        let mut handles = Vec::new();

        for i in 0..8u32 {
            let path = temp_path(&format!("concurrent_{}", i));
            handles.push(tokio::spawn(async move {
                let payload = vec![i as u8; 4096];
                write_file(path.clone(), payload.clone()).await.unwrap();
                let read_back = read_file(path.clone()).await.unwrap();
                assert_eq!(read_back, payload);
                let _ = remove_file(path).await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_overwrite_existing_file() {
        let path = temp_path("overwrite");

        write_file(path.clone(), b"first content".to_vec())
            .await
            .unwrap();
        write_file(path.clone(), b"second".to_vec()).await.unwrap();

        let content = read_file(path.clone()).await.unwrap();
        assert_eq!(content, b"second");

        let _ = remove_file(path).await;
    }

    #[tokio::test]
    async fn test_remove_nonexistent() {
        let path = temp_path("remove_nonexistent_surely");
        let result = remove_file(path).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_streaming_writer_large_payload() {
        let path = temp_path("streaming_large");

        let writer = StreamingWriter::create(path.clone()).await.unwrap();
        let chunk = vec![0xCDu8; 4096];
        for _ in 0..64 {
            writer.write(&chunk).await.unwrap();
        }
        writer.close().await.unwrap();

        let content = read_file(path.clone()).await.unwrap();
        assert_eq!(content.len(), 4096 * 64);
        assert!(content.iter().all(|&b| b == 0xCD));

        let _ = remove_file(path).await;
    }

    // ── io_uring-specific tests (Linux + feature only) ──────────────────

    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    #[tokio::test]
    async fn test_write_read_remove_uring() {
        let _ = init_uring_fs();
        let path = temp_path("uring_raw");
        let payload = vec![0xABu8; 4096]; // 4 KB

        write_file(path.clone(), payload.clone()).await.unwrap();

        let read_back = read_file(path.clone()).await.unwrap();
        assert_eq!(read_back, payload);

        remove_file(path.clone()).await.unwrap();
        assert!(read_file(path).await.is_err());
    }

    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    #[tokio::test]
    async fn test_uring_large_write_read() {
        let _ = init_uring_fs();
        let path = temp_path("uring_large");
        // 2 MB — exercises short-write loop through io_uring.
        let payload = vec![0xFEu8; 2 * 1024 * 1024];

        write_file(path.clone(), payload.clone()).await.unwrap();
        let read_back = read_file(path.clone()).await.unwrap();
        assert_eq!(read_back.len(), payload.len());
        assert_eq!(read_back, payload);

        let _ = remove_file(path).await;
    }

    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    #[tokio::test]
    async fn test_uring_concurrent_ops() {
        let _ = init_uring_fs();
        let mut handles = Vec::new();

        for i in 0..16u32 {
            let path = temp_path(&format!("uring_conc_{}", i));
            handles.push(tokio::spawn(async move {
                let payload = vec![i as u8; 8192];
                write_file(path.clone(), payload.clone()).await.unwrap();
                let read_back = read_file(path.clone()).await.unwrap();
                assert_eq!(read_back, payload);
                let _ = remove_file(path).await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }

    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    #[tokio::test]
    async fn test_uring_empty_file() {
        let _ = init_uring_fs();
        let path = temp_path("uring_empty");

        write_file(path.clone(), Vec::new()).await.unwrap();
        let read_back = read_file(path.clone()).await.unwrap();
        assert!(read_back.is_empty());

        let _ = remove_file(path).await;
    }

    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    #[tokio::test]
    async fn test_uring_overwrite() {
        let _ = init_uring_fs();
        let path = temp_path("uring_overwrite");

        write_file(path.clone(), b"original data here".to_vec())
            .await
            .unwrap();
        write_file(path.clone(), b"replaced".to_vec())
            .await
            .unwrap();

        let content = read_file(path.clone()).await.unwrap();
        assert_eq!(content, b"replaced");

        let _ = remove_file(path).await;
    }

    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    #[tokio::test]
    async fn test_probe_io_uring_does_not_panic() {
        // probe_io_uring must never panic — returns None on unsupported
        // kernels, Some on supported ones.
        let result = super::inner::probe_io_uring();
        // We just verify it didn't panic. The result depends on the
        // kernel, so we don't assert a specific value.
        drop(result);
    }

    // ── TCP connect tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_tcp_connect_fallback() {
        // Bind a local listener, then connect via tcp_connect.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let accept_handle = tokio::spawn(async move { listener.accept().await });
        let connect_handle = tokio::spawn(async move { tcp_connect(addr).await });

        let (accept_result, connect_result) = tokio::join!(accept_handle, connect_handle);

        assert!(accept_result.unwrap().is_ok());
        assert!(
            connect_result.unwrap().is_ok(),
            "tcp_connect should succeed"
        );
    }

    #[tokio::test]
    async fn test_tcp_connect_refused() {
        // Connect to a port that's (almost certainly) not listening.
        let addr: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        let result = tcp_connect(addr).await;
        assert!(result.is_err(), "connecting to port 1 should fail");
    }

    #[tokio::test]
    async fn test_is_uring_enabled_consistent() {
        let e1 = is_uring_enabled();
        let e2 = is_uring_enabled();
        assert_eq!(e1, e2);
    }

    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    #[tokio::test]
    async fn test_tcp_connect_uring() {
        let _ = init_uring_fs();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let accept_handle = tokio::spawn(async move { listener.accept().await });
        let connect_handle = tokio::spawn(async move { tcp_connect(addr).await });

        let (accept_result, connect_result) = tokio::join!(accept_handle, connect_handle);
        assert!(accept_result.unwrap().is_ok());
        assert!(
            connect_result.unwrap().is_ok(),
            "uring tcp_connect should succeed"
        );
    }

    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    #[tokio::test]
    async fn test_tcp_connect_uring_concurrent() {
        let _ = init_uring_fs();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let accept_handle = tokio::spawn(async move {
            let mut streams = Vec::new();
            for _ in 0..8 {
                streams.push(listener.accept().await.unwrap());
            }
            streams
        });

        let mut connect_handles = Vec::new();
        for _ in 0..8 {
            connect_handles.push(tokio::spawn(async move { tcp_connect(addr).await }));
        }

        let _ = accept_handle.await.unwrap();

        for h in connect_handles {
            let result = h.await.unwrap();
            assert!(result.is_ok());
        }
    }

    // ── TCP send/recv tests ──────────────────────────────────────────────

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_tcp_send_recv() {
        use std::os::unix::io::AsRawFd;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Server uses std blocking I/O to avoid deadlocking the single-threaded
        // io_uring worker (both sides competing for the same sequential worker
        // can deadlock if recv is processed before send).
        let std_listener = {
            let l = listener.into_std().unwrap();
            l.set_nonblocking(false).unwrap();
            l
        };

        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = std_listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"hello uring send");
            // Echo back
            stream.write_all(&buf[..n]).unwrap();
        });

        let client_stream = tcp_connect(addr).await.unwrap();
        let fd = client_stream.as_raw_fd();
        let sent = tcp_send(fd, b"hello uring send".to_vec()).await.unwrap();
        assert_eq!(sent, 16);

        let echo = tcp_recv(fd, 1024).await.unwrap();
        assert_eq!(echo, b"hello uring send");

        server.join().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_tcp_send_empty() {
        let sent = tcp_send(0, Vec::new()).await;
        // Empty send on fd 0 — either Ok(0) or error, but must not panic.
        drop(sent);
    }

    // ── Splice tests ────────────────────────────────────────────────────

    #[cfg(all(target_os = "linux", feature = "splice"))]
    #[tokio::test]
    async fn test_splice_socket_to_file() {
        use std::os::unix::io::AsRawFd;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let path = temp_path("splice_out");

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            // Send data that we'll splice on the client side.
            use std::io::Write;
            let std_stream = stream.into_std().unwrap();
            std_stream.set_nonblocking(false).unwrap();
            let mut s = std_stream;
            s.write_all(b"splice test data 1234567890").unwrap();
            drop(s);
        });

        let client_stream = tcp_connect(addr).await.unwrap();
        let socket_fd = client_stream.as_raw_fd();

        // Open a file to splice into.
        let file = std::fs::File::create(&path).unwrap();
        let file_fd = file.as_raw_fd();

        // Wait for data to arrive.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let spliced = splice(socket_fd, file_fd, 27).await.unwrap();
        assert_eq!(spliced, 27);

        drop(file);
        drop(client_stream);
        server.await.unwrap();

        let content = read_file(path.clone()).await.unwrap();
        assert_eq!(content, b"splice test data 1234567890");

        let _ = remove_file(path).await;
    }
}
