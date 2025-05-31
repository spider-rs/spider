/// GPU-related utilities and GPU profile definitions.
pub mod gpu;
/// Constants defining realistic GPU limits such as maximum texture size and resources.
pub mod gpu_limits;
/// GPU profiles tailored specifically for Android devices.
pub mod gpu_android;
/// GPU profiles optimized for desktop and laptop Linux-based operating systems.
pub mod gpu_linux;
/// GPU profiles for Apple macOS systems based on Apple Silicon (M-series).
pub mod gpu_mac;
/// GPU profiles specifically tailored for Windows systems.
pub mod gpu_windows;
/// Defines the common GPU profile structure and utility methods shared across GPU platforms.
pub mod gpu_profile;
