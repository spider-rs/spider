mod builder;
mod event;
pub mod generator;
mod types;

pub use crate::build::generator::{compile_pdls, Generator, SerdeSupport};

pub const CHROMIUM_BASE: &str = "https://chromium.googlesource.com/chromium/src";

// ChromiumDeps = ChromiumBase + "/+/%s/DEPS"
// ChromiumURL  = ChromiumBase +
// "/+/%s/third_party/blink/public/devtools_protocol/browser_protocol.pdl"

pub const V8_BASE: &str = "https://chromium.googlesource.com/v8/v8";

// V8URL  = V8Base + "/+/%s/include/js_protocol.pdl"
//
// // v8 <= 7.6.303.13 uses this path. left for posterity.
// V8URLOld = V8Base + "/+/%s/src/inspector/js_protocol.pdl"
//
// // chromium < 80.0.3978.0 uses this path. left for posterity.
// ChromiumURLOld = ChromiumBase +
// "/+/%s/third_party/blink/renderer/core/inspector/browser_protocol.pdl"
