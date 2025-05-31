pub use super::gpu_android::GPU_PROFILES_ANDROID;
pub use super::gpu_linux::GPU_PROFILES_LINUX;
pub use super::gpu_mac::GPU_PROFILES_MAC;
pub use super::gpu_profile::GpuProfile;
pub use super::gpu_windows::GPU_PROFILES_WINDOWS;

use crate::AgentOs;
use rand::prelude::IndexedRandom;

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
        AgentOs::Linux | AgentOs::Unknown => GPU_PROFILES_LINUX
            .choose(&mut rand::rng())
            .unwrap_or(&FALLBACK_GPU_PROFILE),
        AgentOs::Android => GPU_PROFILES_ANDROID
            .choose(&mut rand::rng())
            .unwrap_or(&FALLBACK_GPU_PROFILE),
    }
}

// FULL Webgl GPU List A-Z
// 3672
// AMD
// ARM
// ATI Technologies Inc.
// Apple
// Apple Inc.
// Google Inc.
// Google Inc. (0x05404C42)
// Google Inc. (AMD)
// Google Inc. (ARM)
// Google Inc. (ATI Technologies Inc.)
// Google Inc. (Apple)
// Google Inc. (Google)
// Google Inc. (Imagination Technologies)
// Google Inc. (Intel Inc.)
// Google Inc. (Intel Open Source Technology Center)
// Google Inc. (Intel)
// Google Inc. (Mesa)
// Google Inc. (Mesa/X.org)
// Google Inc. (Microsoft Corporation)
// Google Inc. (Microsoft)
// Google Inc. (NVIDIA Corporation)
// Google Inc. (NVIDIA Corporation) #7pz9yksmUm
// Google Inc. (NVIDIA Corporation) #NbhL24LYfk
// Google Inc. (NVIDIA Corporation) #SoJg7htQt4
// Google Inc. (NVIDIA Corporation) #udEpwF086L
// Google Inc. (NVIDIA)
// Google Inc. (Qualcomm)
// Google Inc. (Qualcomm) #YVVCJhvoG6
// Google Inc. (Samsung Electronics Co., Ltd.)
// Google Inc.(NVIDIA)
// Imagination Technologies
// Intel
// Intel Inc.
// Intel Open Source Technology Center
// Mesa
// Mesa/X.org
// NA
// NVIDIA Corporation
// Not available
// Qualcomm
// SBdTWBB1Dr
// Samsung Electronics Co., Ltd.
// eHzMdi0sbt
// r0a05FKN
