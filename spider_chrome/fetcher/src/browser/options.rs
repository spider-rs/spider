use std::path::PathBuf;

use directories::BaseDirs;

use crate::error::{FetcherError, Result};
use crate::{Platform, Revision, CURRENT_REVISION};

const CACHE_NAME: &str = "chromiumoxide";
const DEFAULT_HOST: &str = "https://storage.googleapis.com";

/// Options for the fetcher
pub struct BrowserFetcherOptions {
    /// The desired browser revision.
    ///
    /// defaults to CURRENT_REVISION
    pub(crate) revision: Revision,

    /// The host that will be used for downloading.
    ///
    /// defaults to <https://storage.googleapis.com>
    pub(crate) host: String,

    /// The path to download browsers to.
    ///
    /// defaults to $HOME/.cache/chromiumoxide
    pub(crate) path: PathBuf,

    /// The platform to download the browser for.
    ///
    /// defaults to the currently used platform
    pub(crate) platform: Platform,
}

impl BrowserFetcherOptions {
    pub fn builder() -> BrowserFetcherOptionsBuilder {
        BrowserFetcherOptionsBuilder::default()
    }

    #[allow(clippy::should_implement_trait)]
    pub fn default() -> Result<Self> {
        Self::builder().build()
    }
}

#[derive(Default)]
pub struct BrowserFetcherOptionsBuilder {
    revision: Option<Revision>,
    host: Option<String>,
    path: Option<PathBuf>,
    platform: Option<Platform>,
}

impl BrowserFetcherOptionsBuilder {
    pub fn with_revision<T: Into<Revision>>(mut self, revision: T) -> Self {
        self.revision = Some(revision.into());
        self
    }

    pub fn with_host<T: Into<String>>(mut self, host: T) -> Self {
        self.host = Some(host.into());
        self
    }

    pub fn with_path<T: Into<PathBuf>>(mut self, path: T) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_platform<T: Into<Platform>>(mut self, platform: T) -> Self {
        self.platform = Some(platform.into());
        self
    }

    pub fn build(self) -> Result<BrowserFetcherOptions> {
        let path = self
            .path
            .or_else(|| {
                BaseDirs::new().map(|dirs| {
                    let mut path = dirs.cache_dir().to_path_buf();
                    path.push(CACHE_NAME);
                    path
                })
            })
            .ok_or(FetcherError::NoPathAvailable)?;

        let platform =
            self.platform
                .or_else(Platform::current)
                .ok_or(FetcherError::UnsupportedOs(
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                ))?;

        Ok(BrowserFetcherOptions {
            revision: self.revision.unwrap_or(CURRENT_REVISION),
            host: self.host.unwrap_or_else(|| DEFAULT_HOST.to_string()),
            path,
            platform,
        })
    }
}
