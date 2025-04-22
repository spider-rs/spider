/// Chrome utils
#[cfg(feature = "chrome")]
pub mod chrome;
/// Common modules for Chrome
pub mod chrome_common;
#[cfg(feature = "real_browser")]
/// Mouse movements
pub mod chrome_mouse_movements;
/// Chrome spoofing modules
#[cfg(feature = "chrome")]
pub mod chrome_spoof;
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

#[cfg(all(not(feature = "simd"), feature = "openai"))]
pub(crate) use serde_json;
#[cfg(all(feature = "simd", feature = "openai"))]
pub(crate) use sonic_rs as serde_json;
