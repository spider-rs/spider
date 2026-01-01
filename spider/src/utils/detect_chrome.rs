//! Detect Chrome executable path

/// Chrome executable names to search for in PATH
static CHROME_NAMES: &[&str] = &[
    "google-chrome-stable",
    "chromium",
    "google-chrome",
    "chrome",
    "chromium-browser",
    "google-chrome-beta",
    "google-chrome-unstable",
];

/// Relative paths for Chrome executables in home directory
static HOME_PATHS: &[&str] = &[
    "Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ".local/bin/google-chrome-stable",
    ".local/bin/chromium",
    ".local/bin/chrome",
    "bin/google-chrome-stable",
    "bin/chromium",
    "bin/chrome",
];

/// Fallback paths for Chrome executables on different systems
static FALLBACK_PATHS: &[&str] = &[
    "/run/current-system/sw/bin/google-chrome-stable",
    "/run/current-system/sw/bin/chromium",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/google-chrome",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files\\Chromium\\Application\\chrome.exe",
];

/// Get the chrome executable path.
pub fn get_detect_chrome_executable() -> Option<String> {
    // 1. Check CHROME_BIN environment variable
    if let Ok(path) = std::env::var("CHROME_BIN") {
        return Some(path);
    }

    // 2. Check standard executables in PATH using the `which` crate
    for name in CHROME_NAMES {
        if let Ok(path) = which::which(name) {
            return Some(path.to_string_lossy().to_string());
        }
    }

    // 3. Check common paths in HOME directory
    if let Some(home) = home::home_dir() {
        for path_str in HOME_PATHS {
            let path = home.join(path_str);
            if path.exists() {
                return Some(path.to_string_lossy().to_string());
            }
        }
    }

    // 4. Check hardcoded fallback paths (NixOS, MacOS, Linux, Windows)
    for path in FALLBACK_PATHS.iter() {
        let p = std::path::Path::new(path);
        if p.exists() {
            return Some(p.to_string_lossy().to_string());
        }
    }

    None
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_detect_chrome() {
        let path = get_detect_chrome_executable();
        assert!(path.is_some());
        dbg!(path.unwrap());
    }
}
