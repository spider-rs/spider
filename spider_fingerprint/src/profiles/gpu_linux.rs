use super::gpu_profile::GpuProfile;

pub static GPU_PROFILES_LINUX: &[GpuProfile] = &[
    // NVIDIA GTX 1050
    GpuProfile {
        webgl_vendor: "NVIDIA Corporation",
        webgl_renderer: "NVIDIA GeForce GTX 1050/PCIe/SSE2",
        webgpu_vendor: "nvidia",
        webgpu_architecture: "vulkan",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // Intel UHD Graphics 620 with Mesa
    GpuProfile {
        webgl_vendor: "Intel Open Source Technology Center",
        webgl_renderer: "Mesa Intel(R) UHD Graphics 620 (KBL GT2)",
        webgpu_vendor: "intel",
        webgpu_architecture: "vulkan",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // AMD Radeon RX 580 with Mesa
    GpuProfile {
        webgl_vendor: "AMD",
        webgl_renderer:
            "AMD Radeon RX 580 Series (POLARIS10, DRM 3.42.0, 6.2.12-arch1-1, LLVM 15.0.7)",
        webgpu_vendor: "amd",
        webgpu_architecture: "vulkan",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 12,
    },
    // NVIDIA RTX 3060 with proprietary driver
    GpuProfile {
        webgl_vendor: "NVIDIA Corporation",
        webgl_renderer: "NVIDIA GeForce RTX 3060/PCIe/SSE2",
        webgpu_vendor: "nvidia",
        webgpu_architecture: "vulkan",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 16,
    },
    // Intel Arc A770 (modern Intel GPU)
    GpuProfile {
        webgl_vendor: "Intel Open Source Technology Center",
        webgl_renderer: "Mesa Intel Arc A770 Graphics (DG2)",
        webgpu_vendor: "intel",
        webgpu_architecture: "vulkan",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 16,
    },
    // Additional exact data derived from provided prompt:
    // Mesa/X.org (Common Linux open-source default renderer using LLVMpipe)
    GpuProfile {
        webgl_vendor: "Mesa/X.org",
        webgl_renderer: "llvmpipe (LLVM 15.0.7, 256 bits)",
        webgpu_vendor: "mesa",
        webgpu_architecture: "vulkan",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 4,
    },
    // Mesa (Open-source OpenGL implementation, software rendering fallback)
    GpuProfile {
        webgl_vendor: "Mesa",
        webgl_renderer: "llvmpipe (LLVM 15.0.7, 256 bits)",
        webgpu_vendor: "mesa",
        webgpu_architecture: "vulkan",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 4,
    },
    // Intel Inc Integrated Graphics common variant
    GpuProfile {
        webgl_vendor: "Intel Open Source Technology Center",
        webgl_renderer: "Mesa Intel(R) HD Graphics 520 (SKL GT2)",
        webgpu_vendor: "intel",
        webgpu_architecture: "vulkan",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 4,
    },
    // AMD integrated GPU (e.g., Ryzen integrated graphics)
    GpuProfile {
        webgl_vendor: "AMD",
        webgl_renderer: "AMD Radeon Graphics (RENOIR, DRM 3.40.0, 5.10.0-8-amd64, LLVM 11.0.1)",
        webgpu_vendor: "amd",
        webgpu_architecture: "vulkan",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // NVIDIA legacy profile (GTX 1080 variant)
    GpuProfile {
        webgl_vendor: "NVIDIA Corporation",
        webgl_renderer: "NVIDIA GeForce GTX 1080/PCIe/SSE2",
        webgpu_vendor: "nvidia",
        webgpu_architecture: "vulkan",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
];
