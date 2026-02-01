/// Chrome utils
#[cfg(feature = "chrome")]
pub mod chrome;
#[cfg(feature = "chrome")]
/// Chrome launch args.
pub(crate) mod chrome_args;
/// Common modules for Chrome
pub mod chrome_common;
#[cfg(feature = "real_browser")]
/// Viewport
pub mod chrome_viewport;

/// WebDriver utils
#[cfg(feature = "webdriver")]
pub mod webdriver;
#[cfg(feature = "webdriver")]
/// WebDriver launch args.
pub(crate) mod webdriver_args;
/// Common modules for WebDriver
pub mod webdriver_common;

/// Decentralized header handling
#[cfg(feature = "decentralized_headers")]
pub mod decentralized_headers;
/// Disk options
pub mod disk;
/// URL globbing
#[cfg(feature = "glob")]
pub mod glob;
/// OpenAI
#[cfg(feature = "openai")]
pub mod openai;
/// Common modules for OpenAI
pub mod openai_common;

/// Gemini
#[cfg(feature = "gemini")]
pub mod gemini;
/// Common modules for Gemini
pub mod gemini_common;

/// Solve all.
pub mod solvers;

#[cfg(all(not(feature = "simd"), feature = "openai"))]
pub(crate) use serde_json;
#[cfg(all(feature = "simd", feature = "openai"))]
pub(crate) use sonic_rs as serde_json;

/// Automation scripts.
pub mod automation;
