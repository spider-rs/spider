use super::gpu_profile::GpuProfile;

pub static GPU_PROFILES_ANDROID: &[GpuProfile] = &[
    // Google Pixel 6 / 6a (ARM Mali-G78)
    GpuProfile {
        webgl_vendor: "ARM",
        webgl_renderer: "Mali-G78",
        webgpu_vendor: "arm",
        webgpu_architecture: "opengl",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // Samsung Galaxy S22 (Adreno 730)
    GpuProfile {
        webgl_vendor: "Qualcomm",
        webgl_renderer: "Adreno (TM) 730",
        webgpu_vendor: "qualcomm",
        webgpu_architecture: "opengl",
        canvas_format: "rgba8unorm",
        hardware_concurrency: 8,
    },
    // Samsung Galaxy A52 (Adreno 618)
    GpuProfile {
        webgl_vendor: "Qualcomm",
        webgl_renderer: "Adreno (TM) 618",
        webgpu_vendor: "qualcomm",
        webgpu_architecture: "opengl",
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
