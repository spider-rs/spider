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
    /// The hardware concurrency limits
    pub hardware_concurrency: usize,
}

pub static GPU_PROFILES_MAC: &[GpuProfile] = &[
    // Apple M1 (MacBook Air/Pro base models)
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M1, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 8,
    },
    // Apple M1 Max
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M1 Max, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 10,
    },
    // Apple M2
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M2, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 8,
    },
    // Apple M3 (base model)
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M3, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 8,
    },
    // Apple M3 Pro (11-core or 12-core variants)
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M3 Pro, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 12,
    },
    // Apple M3 Max
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M3 Max, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 16,
    },
];

pub static GPU_PROFILES_WINDOWS: &[GpuProfile] = &[
    GpuProfile {
        webgl_vendor: "Google Inc.",
        webgl_renderer: "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060, D3D11)",
        webgpu_vendor: "Google Inc. (NVIDIA)",
        webgpu_architecture: "",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 10,
    },
    GpuProfile {
        webgl_vendor: "Google Inc.",
        webgl_renderer: "ANGLE (Intel, Intel(R) Iris(R) Xe Graphics, D3D11)",
        webgpu_vendor: "Google Inc. (NVIDIA)",
        webgpu_architecture: "",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 10,
    },
];

pub static GPU_PROFILES_LINUX: &[GpuProfile] = &[
    // NVIDIA GTX 1050 (your existing one)
    GpuProfile {
        webgl_vendor: "Google Inc.",
        webgl_renderer: "ANGLE (NVIDIA, NVIDIA GeForce GTX 1050 Direct3D11 vs_5_0 ps_5_0, D3D11-27.21.14.5671)",
        webgpu_vendor: "Google Inc. (NVIDIA)",
        webgpu_architecture: "",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 10,
    },
    // Intel UHD Graphics 620 (common on laptops)
    GpuProfile {
        webgl_vendor: "Google Inc.",
        webgl_renderer: "ANGLE (Intel, Intel(R) UHD Graphics 620 Direct3D11 vs_5_0 ps_5_0, D3D11-25.20.100.6446)",
        webgpu_vendor: "Google Inc. (Intel)",
        webgpu_architecture: "",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // AMD Radeon RX 580
    GpuProfile {
        webgl_vendor: "Google Inc.",
        webgl_renderer: "ANGLE (AMD, AMD Radeon RX 580 Series Direct3D11 vs_5_0 ps_5_0, D3D11-26.20.15015.1007)",
        webgpu_vendor: "Google Inc. (AMD)",
        webgpu_architecture: "",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 12,
    },
    // NVIDIA RTX 3060 (newer high-end profile)
    GpuProfile {
        webgl_vendor: "Google Inc.",
        webgl_renderer: "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11-30.0.15.1179)",
        webgpu_vendor: "Google Inc. (NVIDIA)",
        webgpu_architecture: "",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 16,
    },
    // Intel Arc A770 (modern Intel GPU)
    GpuProfile {
        webgl_vendor: "Google Inc.",
        webgl_renderer: "ANGLE (Intel, Intel(R) Arc(TM) A770 Graphics Direct3D11 vs_5_0 ps_5_0, D3D11-31.0.101.4644)",
        webgpu_vendor: "Google Inc. (Intel)",
        webgpu_architecture: "",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 16,
    },
];

pub static GPU_PROFILES_ANDROID: &[GpuProfile] = &[
    // Google Pixel 6 / 6a (ARM Mali-G78)
    GpuProfile {
        webgl_vendor: "ARM",
        webgl_renderer: "Mali-G78",
        webgpu_vendor: "ARM",
        webgpu_architecture: "mali-g78",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // Samsung Galaxy S22 (Qualcomm Adreno 730)
    GpuProfile {
        webgl_vendor: "Qualcomm",
        webgl_renderer: "Adreno (TM) 730",
        webgpu_vendor: "Qualcomm",
        webgpu_architecture: "adreno-730",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // Samsung Galaxy A52 (Adreno 618)
    GpuProfile {
        webgl_vendor: "Qualcomm",
        webgl_renderer: "Adreno (TM) 618",
        webgpu_vendor: "Qualcomm",
        webgpu_architecture: "adreno-618",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // Xiaomi Redmi Note 11 (ARM Mali-G57)
    GpuProfile {
        webgl_vendor: "ARM",
        webgl_renderer: "Mali-G57",
        webgpu_vendor: "ARM",
        webgpu_architecture: "mali-g57",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // OnePlus 9 Pro (Adreno 660)
    GpuProfile {
        webgl_vendor: "Qualcomm",
        webgl_renderer: "Adreno (TM) 660",
        webgpu_vendor: "Qualcomm",
        webgpu_architecture: "adreno-660",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // Google Pixel 5 (Adreno 620)
    GpuProfile {
        webgl_vendor: "Qualcomm",
        webgl_renderer: "Adreno (TM) 620",
        webgpu_vendor: "Qualcomm",
        webgpu_architecture: "adreno-620",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
];

/// Fallback GPU profile used when no valid match is found.
pub static FALLBACK_GPU_PROFILE: GpuProfile = GpuProfile {
    webgl_vendor: "Google Inc.",
    webgl_renderer: "ANGLE (Unknown, Generic Renderer, OpenGL)",
    webgpu_vendor: "Google Inc. (NVIDIA)",
    webgpu_architecture: "",
    canvas_format: "rgba8unorm",
    hardware_concurrency: 10,
};

/// Select a random GPU profile.
pub fn select_random_gpu_profile(os: crate::AgentOs) -> &'static GpuProfile {
    match os {
        AgentOs::Mac => GPU_PROFILES_MAC
            .choose(&mut rand::rng())
            .unwrap_or(&FALLBACK_GPU_PROFILE),
        AgentOs::Windows => GPU_PROFILES_WINDOWS
            .choose(&mut rand::rng())
            .unwrap_or(&FALLBACK_GPU_PROFILE),
        AgentOs::Linux => GPU_PROFILES_LINUX
            .choose(&mut rand::rng())
            .unwrap_or(&FALLBACK_GPU_PROFILE),
        AgentOs::Android => GPU_PROFILES_ANDROID
            .choose(&mut rand::rng())
            .unwrap_or(&FALLBACK_GPU_PROFILE),
    }
}
