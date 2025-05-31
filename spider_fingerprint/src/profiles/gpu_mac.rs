use super::gpu_profile::GpuProfile;

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
    // Apple M1 Pro (MacBook Pro 14/16-inch)
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M1 Pro, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 10,
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
    // Apple M1 Ultra (Mac Studio)
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M1 Ultra, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 20,
    },
    // Apple M2 (MacBook Air, Mac mini)
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M2, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 8,
    },
    // Apple M2 Pro (MacBook Pro, Mac mini)
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M2 Pro, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 12,
    },
    // Apple M2 Max (MacBook Pro, Mac Studio)
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M2 Max, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 16,
    },
    // Apple M2 Ultra (Mac Studio, Mac Pro)
    GpuProfile {
        webgl_vendor: "Google Inc. (Apple)",
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M2 Ultra, Unspecified Version)",
        webgpu_vendor: "apple",
        webgpu_architecture: "metal-3",
        canvas_format: "bgra8unorm",
        hardware_concurrency: 24,
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
    // Apple M3 Pro (11-core or 12-core)
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
