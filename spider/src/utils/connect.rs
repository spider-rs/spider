use pin_project_lite::pin_project;
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::AtomicBool,
    task::{Context, Poll},
};
use tokio::{
    select,
    sync::{mpsc::error::SendError, OnceCell},
};
use tower::{BoxError, Layer, Service};

/// A threadpool dedicated for connecting to services.
static CONNECT_THREAD_POOL: OnceCell<
    tokio::sync::mpsc::UnboundedSender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
> = OnceCell::const_new();

/// Is the background thread connect enabled.
static BACKGROUND_THREAD_CONNECT_ENABLED: AtomicBool = AtomicBool::new(true);

/// Is the background thread initialized and enabled.
#[allow(dead_code)]
pub(crate) fn background_connect_threading() -> bool {
    BACKGROUND_THREAD_CONNECT_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Init a background thread for request connect handling.
#[cfg(all(target_os = "linux", feature = "io_uring"))]
pub fn init_background_runtime() {
    let _ = CONNECT_THREAD_POOL.set({
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let builder = std::thread::Builder::new();

        if let Err(_) = builder.spawn(move || {
            tokio_uring::builder().start(async {
                while let Some(work) = rx.recv().await {
                    tokio_uring::spawn(work);
                }
            })
        }) {
            let _ = tx.downgrade();
            BACKGROUND_THREAD_CONNECT_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
        };

        tx
    });
}

/// Init a background thread for request connect handling.
#[cfg(any(not(target_os = "linux"), not(feature = "io_uring")))]
pub fn init_background_runtime() {
    let _ = CONNECT_THREAD_POOL.set({
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let builder = std::thread::Builder::new();

        if builder
            .spawn(move || {
                match tokio::runtime::Builder::new_multi_thread()
                    .thread_name("connect-background-pool-thread")
                    .worker_threads(num_cpus::get())
                    .on_thread_start(move || {
                        #[cfg(target_os = "linux")]
                        unsafe {
                            if libc::nice(10) == -1 && *libc::__errno_location() != 0 {
                                let error = std::io::Error::last_os_error();
                                log::error!("failed to set threadpool niceness: {}", error);
                            }
                        }
                    })
                    .enable_all()
                    .build()
                {
                    Ok(rt) => {
                        rt.block_on(async move {
                            while let Some(work) = rx.recv().await {
                                tokio::task::spawn(work);
                            }
                        });
                    }
                    _ => {
                        BACKGROUND_THREAD_CONNECT_ENABLED
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            })
            .is_err()
        {
            let _ = tx.downgrade();
            BACKGROUND_THREAD_CONNECT_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
        };

        tx
    });
}

/// This tower layer injects futures with a oneshot channel, and then sends them to the background runtime for processing.
#[derive(Copy, Clone)]
pub struct BackgroundProcessorLayer;

impl Default for BackgroundProcessorLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundProcessorLayer {
    /// A new background proccess layer shortcut.
    pub fn new() -> Self {
        Self
    }
}
impl<S> Layer<S> for BackgroundProcessorLayer {
    type Service = BackgroundProcessor<S>;
    fn layer(&self, service: S) -> Self::Service {
        BackgroundProcessor::new(service)
    }
}

impl std::fmt::Debug for BackgroundProcessorLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("BackgroundProcessorLayer").finish()
    }
}

/// Send to the background runtime.
fn send_to_background_runtime(future: impl Future<Output = ()> + Send + 'static) {
    let tx = CONNECT_THREAD_POOL.get().expect(
        "background runtime should be initialized by calling init_background_runtime before use",
    );

    if let Err(SendError(_)) = tx.send(Box::pin(future)) {
        log::error!("Failed to send future - background connect runtime channel is closed. Abandoning task.");
    }
}

/// This tower service injects futures with a oneshot channel, and then sends them to the background runtime for processing.
#[derive(Debug, Clone)]
pub struct BackgroundProcessor<S> {
    inner: S,
}

impl<S> BackgroundProcessor<S> {
    /// Setup a new connect background processor.
    pub fn new(inner: S) -> Self {
        BackgroundProcessor { inner }
    }
}

impl<S, Request> Service<Request> for BackgroundProcessor<S>
where
    S: Service<Request>,
    S::Response: Send + 'static,
    S::Error: Into<BoxError> + Send,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = BackgroundResponseFuture<S::Response>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        match self.inner.poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(r) => Poll::Ready(r.map_err(Into::into)),
        }
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let response = self.inner.call(req);
        let (mut tx, rx) = tokio::sync::oneshot::channel();

        let future = async move {
            select! {
                _ = tx.closed() => (),
                result = response => {
                    let _ = tx.send(result.map_err(Into::into));
                }
            }
        };

        send_to_background_runtime(future);
        BackgroundResponseFuture::new(rx)
    }
}

pin_project! {
    #[derive(Debug)]
    /// A new background response future.
    pub struct BackgroundResponseFuture<S> {
        #[pin]
        rx: tokio::sync::oneshot::Receiver<Result<S, BoxError>>,
    }
}

impl<S> BackgroundResponseFuture<S> {
    pub(crate) fn new(rx: tokio::sync::oneshot::Receiver<Result<S, BoxError>>) -> Self {
        BackgroundResponseFuture { rx }
    }
}

impl<S> Future for BackgroundResponseFuture<S>
where
    S: Send + 'static,
{
    type Output = Result<S, BoxError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.rx.poll(cx) {
            Poll::Ready(v) => match v {
                Ok(v) => Poll::Ready(v),
                Err(err) => Poll::Ready(Err(Box::new(err) as BoxError)),
            },
            Poll::Pending => Poll::Pending,
        }
    }
}
