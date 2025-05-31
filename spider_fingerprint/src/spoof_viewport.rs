use rand::distr::{weighted::WeightedIndex, Distribution};
use rand::prelude::IndexedRandom;
use rand::rngs::ThreadRng;
use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// View port handling for chrome.
pub struct Viewport {
    /// Device screen Width
    pub width: u32,
    /// Device screen size
    pub height: u32,
    /// Device scale factor
    pub device_scale_factor: Option<f64>,
    /// Emulating Mobile?
    pub emulating_mobile: bool,
    /// Use landscape mode instead of portrait.
    pub is_landscape: bool,
    /// Touch screen device?
    pub has_touch: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            width: 800,
            height: 600,
            device_scale_factor: None,
            emulating_mobile: false,
            is_landscape: false,
            has_touch: false,
        }
    }
}

impl Viewport {
    /// Create a new viewport layout for chrome passing in the width.
    pub fn new(width: u32, height: u32) -> Self {
        Viewport {
            width,
            height,
            ..Default::default()
        }
    }
    /// Determine if the layout is a mobile device or not to emulate.
    pub fn set_mobile(&mut self, emulating_mobile: bool) {
        self.emulating_mobile = emulating_mobile;
    }
    /// Determine if the layout is in landscrape view or not to emulate.
    pub fn set_landscape(&mut self, is_landscape: bool) {
        self.is_landscape = is_landscape;
    }
    /// Determine if the device is a touch screen or not to emulate.
    pub fn set_touch(&mut self, has_touch: bool) {
        self.has_touch = has_touch;
    }
    /// The scale factor for the screen layout.
    pub fn set_scale_factor(&mut self, device_scale_factor: Option<f64>) {
        self.device_scale_factor = device_scale_factor;
    }
}

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
    randomize_viewport_rng(device, &mut rand::rng())
}

/// Randomize viewport dimensions based on device type.
pub fn randomize_viewport_rng(device: &DeviceType, rng: &mut ThreadRng) -> Viewport {
    match device {
        DeviceType::Mobile => {
            let width = rng.random_range(320..=480);
            let height = rng.random_range(480..=800);
            Viewport::new(width, height)
        }
        DeviceType::Tablet => {
            let width = rng.random_range(600..=800);
            let height = rng.random_range(800..=1200);
            Viewport::new(width, height)
        }
        DeviceType::Desktop => {
            let width = rng.random_range(1024..=1920);
            let height = rng.random_range(768..=1080);
            Viewport::new(width, height)
        }
    }
}

/// Get a random viewport by selecting a random device type first. The weights are aligned in favor of desktop.
pub fn get_random_viewport() -> Viewport {
    get_random_viewport_rng(&mut rand::rng())
}

/// Get a random viewport by selecting a random device type first. The weights are aligned in favor of desktop.
pub fn get_random_viewport_rng(rng: &mut ThreadRng) -> Viewport {
    let device_types = [DeviceType::Mobile, DeviceType::Tablet, DeviceType::Desktop];

    randomize_viewport(if let Ok(dist) = WeightedIndex::new(&[1, 1, 3]) {
        &device_types[dist.sample(rng)]
    } else {
        device_types.choose(rng).unwrap_or(&DeviceType::Desktop)
    })
}
