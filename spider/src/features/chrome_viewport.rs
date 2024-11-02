use crate::features::chrome_common::Viewport;
use rand::distributions::{Distribution, WeightedIndex};
use rand::prelude::SliceRandom;
use rand::Rng;

/// Represents different types of devices for viewport simulation.
///
/// The `DeviceType` enum categorizes devices into three main types:
/// `Mobile`, `Tablet`, and `Desktop`. These categories are typically used
/// to simulate and test varying screen resolutions and viewports in
/// web development, ensuring that web applications render correctly
/// across different platforms.
#[derive(Default, Clone, Copy, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DeviceType {
    /// Represents a mobile device.
    ///
    /// Mobile devices generally have smaller screens, and their
    /// viewport sizes range from small to medium dimensions, typically
    /// used for smartphones.
    #[cfg_attr(feature = "serde", serde(rename = "mobile"))]
    Mobile,
    /// Represents a tablet device.
    ///
    /// Tablet devices have medium-sized screens, which are larger than
    /// those of mobile devices but smaller than desktop monitors.
    /// Viewport dimensions for tablets are typically larger than mobiles
    /// and intended for medium-scale interfaces.
    #[cfg_attr(feature = "serde", serde(rename = "tablet"))]
    Tablet,
    #[default]
    /// Represents a desktop device.
    ///
    /// Desktop devices have larger screens compared to mobile and
    /// tablet devices. Viewport dimensions for desktops are typically
    /// the largest among the three, intended for full-scale applications
    /// on monitors.
    #[cfg_attr(feature = "serde", serde(rename = "desktop"))]
    Desktop,
}

/// Randomize viewport dimensions based on device type
pub fn randomize_viewport(device: &DeviceType) -> Viewport {
    let mut rng = rand::thread_rng();

    match device {
        DeviceType::Mobile => {
            let width = rng.gen_range(320..=480);
            let height = rng.gen_range(480..=800);
            Viewport::new(width, height)
        }
        DeviceType::Tablet => {
            let width = rng.gen_range(600..=800);
            let height = rng.gen_range(800..=1200);
            Viewport::new(width, height)
        }
        DeviceType::Desktop => {
            let width = rng.gen_range(1024..=1920);
            let height = rng.gen_range(768..=1080);
            Viewport::new(width, height)
        }
    }
}

/// Get a random viewport by selecting a random device type first. The weights are aligned in favor of desktop.
pub fn get_random_viewport() -> Viewport {
    let mut rng = rand::thread_rng();
    let device_types = [DeviceType::Mobile, DeviceType::Tablet, DeviceType::Desktop];

    randomize_viewport(if let Ok(dist) = WeightedIndex::new(&[1, 1, 3]) {
        &device_types[dist.sample(&mut rng)]
    } else {
        device_types
            .choose(&mut rng)
            .unwrap_or(&DeviceType::Desktop)
    })
}
