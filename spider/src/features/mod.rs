/// Chrome utils
#[cfg(feature = "chrome")]
pub mod chrome;
/// Common modules for Chrome
pub mod chrome_common;
#[cfg(feature = "real_browser")]
/// Mouse movements
pub mod chrome_mouse_movements;
#[cfg(feature = "real_browser")]
/// Viewport
pub mod chrome_viewport;
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
/// Spoof the refereer
pub mod spoof_referrer;

lazy_static::lazy_static! {
    /// The max links to store in memory.
    pub(crate) static ref LINKS_VISITED_MEMORY_LIMIT: usize = {
        const DEFAULT_LIMIT: usize = 15_000;

        match std::env::var("LINKS_VISITED_MEMORY_LIMIT") {
            Ok(limit) => limit.parse::<usize>().unwrap_or(DEFAULT_LIMIT),
            _ => DEFAULT_LIMIT
        }
    };
}
