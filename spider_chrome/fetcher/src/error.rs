use std::num::ParseIntError;

use thiserror::Error;

pub type Result<T, E = FetcherError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum FetcherError {
    #[error("Invalid browser revision")]
    InvalidRevision(#[source] ParseIntError),

    #[error("No path available to download browsers to")]
    NoPathAvailable,

    #[error("Download of browser failed")]
    DownloadFailed(#[source] anyhow::Error),

    #[error("Installation of browser failed")]
    InstallFailed(#[source] anyhow::Error),

    #[error("OS {0} {1} is not supported")]
    UnsupportedOs(&'static str, &'static str),
}
