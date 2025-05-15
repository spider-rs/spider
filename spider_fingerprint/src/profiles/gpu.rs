use crate::AgentOs;
use rand::prelude::IndexedRandom;

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
}

pub static GPU_PROFILES_MAC: &[GpuProfile] = &[
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M1, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
    },
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M2, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
    },
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M3, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
    },
];

pub static GPU_PROFILES_WINDOWS: &[GpuProfile] = &[
    GpuProfile {
        webgl_vendor: "Google Inc.",
        webgl_renderer: "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060, D3D11)",
        webgpu_vendor: "Google Inc. (NVIDIA)",
        webgpu_architecture: "",
        canvas_format: "rgba8unorm",
    },
    GpuProfile {
        webgl_vendor: "Google Inc.",
        webgl_renderer: "ANGLE (Intel, Intel(R) Iris(R) Xe Graphics, D3D11)",
        webgpu_vendor: "Google Inc. (NVIDIA)",
        webgpu_architecture: "",
        canvas_format: "rgba8unorm",
    },
];

/// Select a random GPU profile.
pub fn select_random_gpu_profile(os: crate::AgentOs) -> &'static GpuProfile {
    match os {
        AgentOs::Mac => GPU_PROFILES_MAC.choose(&mut rand::rng()).unwrap(),
        AgentOs::Windows => GPU_PROFILES_WINDOWS.choose(&mut rand::rng()).unwrap(),
        _ => &GpuProfile {
            webgl_vendor: "Google Inc.",
            webgl_renderer: "ANGLE (Unknown, Generic Renderer, OpenGL)",
            webgpu_vendor: "Google Inc. (NVIDIA)",
            webgpu_architecture: "",
            canvas_format: "rgba8unorm",
        },
    }
}
