use pin_project_lite::pin_project;
use std::{
    future::Future,
    pin::Pin,
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

/// Init a background thread for request connect handling.
pub(crate) async fn init_background_runtime() {
    CONNECT_THREAD_POOL
        .get_or_init(|| async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .thread_name("connect-background-pool-thread")
                    .worker_threads(num_cpus::get() as usize)
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
                    .unwrap_or_else(|e| panic!("connect runtime failed_to_initialize: {}", e));

                rt.block_on(async move {
                    while let Some(work) = rx.recv().await {
                        tokio::task::spawn(work);
                    }
                });
            });

            tx
        })
        .await;
}

/// This tower layer injects futures with a oneshot channel, and then sends them to the background runtime for processing.
#[derive(Copy, Clone)]
pub struct BackgroundProcessorLayer;

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
    // Retrieve the sender from the OnceCell. This will create the sender lazily if it wasn't already.
    let tx = CONNECT_THREAD_POOL.get().expect(
        "background runtime should be initialized by calling init_background_runtime before use",
    );

    match tx.send(Box::pin(future)) {
        Ok(_) => (),
        Err(SendError(_)) => {
            log::error!("Failed to send future - background connect runtime channel is closed. Abandoning task.");
        }
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
                Ok(v) => Poll::Ready(v.map_err(Into::into)),
                Err(err) => Poll::Ready(Err(Box::new(err) as BoxError)),
            },
            Poll::Pending => Poll::Pending,
        }
    }
}
