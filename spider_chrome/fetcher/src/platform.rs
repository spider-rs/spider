use std::path::{Path, PathBuf};

use crate::Revision;

/// List of platforms with pre-built chromium binaries
#[derive(Clone, Copy, Debug)]
pub enum Platform {
    Linux,
    Mac,
    MacArm,
    Win32,
    Win64,
}

impl Platform {
    #[doc(hidden)] // internal API
    pub fn download_url(&self, host: &str, revision: &Revision) -> String {
        let archive = self.archive_name(revision);
        let name = match self {
            Self::Linux => "Linux_x64",
            Self::Mac => "Mac",
            Self::MacArm => "Mac_Arm",
            Self::Win32 => "Win",
            Self::Win64 => "Win_x64",
        };
        format!(
            "{}/chromium-browser-snapshots/{}/{}/{}.zip",
            host, name, revision, archive
        )
    }

    pub(crate) fn archive_name(&self, revision: &Revision) -> String {
        match self {
            Self::Linux => "chrome-linux".to_string(),
            Self::Mac | Self::MacArm => "chrome-mac".to_string(),
            Self::Win32 | Self::Win64 => {
                if revision.0 > 591_479 {
                    "chrome-win".to_string()
                } else {
                    "chrome-win32".to_string()
                }
            }
        }
    }

    pub(crate) fn folder_name(&self, revision: &Revision) -> String {
        let platform = match self {
            Self::Linux => "linux",
            Self::Mac => "mac",
            Self::MacArm => "mac_arm",
            Self::Win32 => "win32",
            Self::Win64 => "win64",
        };
        format!("{platform}-{revision}")
    }

    pub(crate) fn executable(&self, folder_path: &Path, revision: &Revision) -> PathBuf {
        let mut path = folder_path.to_path_buf();
        path.push(self.archive_name(revision));
        match self {
            Self::Linux => path.push("chrome"),
            Self::Mac | Self::MacArm => {
                path.push("Chromium.app");
                path.push("Contents");
                path.push("MacOS");
                path.push("Chromium")
            }
            Self::Win32 | Self::Win64 => path.push("chrome.exe"),
        }
        path
    }

    pub(crate) fn current() -> Option<Platform> {
        // Currently there are no builds for Linux arm
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Some(Self::Linux)
        } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            Some(Self::Mac)
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Some(Self::MacArm)
        } else if cfg!(all(target_os = "windows", target_arch = "x86")) {
            Some(Self::Win32)
        } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Some(Self::Win64)
        } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
            // x64 emulation is available for windows 11
            if let os_info::Version::Semantic(major, _, _) = os_info::get().version() {
                if *major > 10 {
                    return Some(Self::Win64);
                }
            }
            None
        } else {
            None
        }
    }
}
