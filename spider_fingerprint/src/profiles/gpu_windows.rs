use super::gpu_profile::GpuProfile;

pub static GPU_PROFILES_WINDOWS: &[GpuProfile] = &[
    // NVIDIA RTX 3060
    GpuProfile {
        webgl_vendor: "Google Inc. (NVIDIA Corporation)",
        webgl_renderer: "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060, D3D11)",
        webgpu_vendor: "nvidia",
        webgpu_architecture: "d3d11",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 12,
    },
    // NVIDIA RTX 4090
    GpuProfile {
        webgl_vendor: "Google Inc. (NVIDIA Corporation)",
        webgl_renderer: "ANGLE (NVIDIA, NVIDIA GeForce RTX 4090, D3D11)",
        webgpu_vendor: "nvidia",
        webgpu_architecture: "d3d12",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 24,
    },
    // NVIDIA GTX 1650
    GpuProfile {
        webgl_vendor: "Google Inc. (NVIDIA Corporation)",
        webgl_renderer: "ANGLE (NVIDIA, NVIDIA GeForce GTX 1650, D3D11)",
        webgpu_vendor: "nvidia",
        webgpu_architecture: "d3d11",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // Intel Iris Xe
    GpuProfile {
        webgl_vendor: "Google Inc. (Intel)",
        webgl_renderer: "ANGLE (Intel, Intel(R) Iris(R) Xe Graphics, D3D11)",
        webgpu_vendor: "intel",
        webgpu_architecture: "d3d11",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // Intel UHD Graphics 630
    GpuProfile {
        webgl_vendor: "Google Inc. (Intel)",
        webgl_renderer: "ANGLE (Intel, Intel(R) UHD Graphics 630, D3D11)",
        webgpu_vendor: "intel",
        webgpu_architecture: "d3d11",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 6,
    },
    // AMD Radeon RX 6700 XT
    GpuProfile {
        webgl_vendor: "Google Inc. (AMD)",
        webgl_renderer: "ANGLE (AMD, AMD Radeon RX 6700 XT, D3D11)",
        webgpu_vendor: "amd",
        webgpu_architecture: "d3d11",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 12,
    },
    // AMD Radeon RX 5600 XT
    GpuProfile {
        webgl_vendor: "Google Inc. (AMD)",
        webgl_renderer: "ANGLE (AMD, AMD Radeon RX 5600 XT, D3D11)",
        webgpu_vendor: "amd",
        webgpu_architecture: "d3d11",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 12,
    },
    // AMD Radeon RX Vega 10 Graphics (integrated GPU, common on Ryzen laptops)
    GpuProfile {
        webgl_vendor: "Google Inc. (AMD)",
        webgl_renderer: "ANGLE (AMD, AMD Radeon RX Vega 10 Graphics, D3D11)",
        webgpu_vendor: "amd",
        webgpu_architecture: "d3d11",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // NVIDIA RTX 2060
    GpuProfile {
        webgl_vendor: "Google Inc. (NVIDIA Corporation)",
        webgl_renderer: "ANGLE (NVIDIA, NVIDIA GeForce RTX 2060, D3D11)",
        webgpu_vendor: "nvidia",
        webgpu_architecture: "d3d11",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 12,
    },
    // Intel Arc A750 Graphics
    GpuProfile {
        webgl_vendor: "Google Inc. (Intel)",
        webgl_renderer: "ANGLE (Intel, Intel(R) Arc(TM) A750 Graphics, D3D12)",
        webgpu_vendor: "intel",
        webgpu_architecture: "d3d12",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 16,
    },
];
