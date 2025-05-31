/// A profile that defines GPU-related fingerprinting properties for spoofing
/// WebGL and WebGPU APIs.
///
/// This structure can be used to generate consistent spoofed values across
/// browser environments, including workers. It includes values that map to
/// `WebGLRenderingContext.getParameter`, `navigator.gpu`, and
/// `navigator.gpu.requestAdapter().then(...)`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GpuProfile {
    /// The spoofed value for `UNMASKED_VENDOR_WEBGL` (GL enum 37445).
    /// Typically returned by `gl.getParameter(...)` from WebGL.
    pub webgl_vendor: &'static str,
    /// The spoofed value for `UNMASKED_RENDERER_WEBGL` (GL enum 37446).
    /// Represents the WebGL renderer string.
    pub webgl_renderer: &'static str,
    /// The spoofed value for `navigator.gpu.requestAdapter().then(a => a.info.vendor)`.
    /// This usually matches the GPU vendor in lowercase (e.g., `"apple"`, `"nvidia"`).
    pub webgpu_vendor: &'static str,
    /// The spoofed architecture value from `navigator.gpu.requestAdapter().then(a => a.info.architecture)`.
    /// Example: `"metal-3"`, `"d3d11"`, `"opengl"` etc.
    pub webgpu_architecture: &'static str,
    /// The value returned by `navigator.gpu.getPreferredCanvasFormat()`.
    /// Chrome on macOS returns `"bgra8unorm"`, others return `"rgba8unorm"`.
    pub canvas_format: &'static str,
    /// The hardware concurrency limits
    pub hardware_concurrency: usize,
}
