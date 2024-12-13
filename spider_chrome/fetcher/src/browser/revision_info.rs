use std::fmt;
use std::path::PathBuf;

use crate::Revision;

/// Details of an installed version of chromium
#[derive(Clone, Debug)]
pub struct BrowserFetcherRevisionInfo {
    pub folder_path: PathBuf,
    pub executable_path: PathBuf,
    pub revision: Revision,
}

impl fmt::Display for BrowserFetcherRevisionInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Revision: {}, Path: {}",
            self.revision,
            self.executable_path.display()
        )
    }
}
