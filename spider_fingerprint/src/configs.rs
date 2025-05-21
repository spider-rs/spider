/// Tier of stealth to use.
#[derive(PartialEq, Debug, Default, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Tier {
    #[default]
    /// Basic spoofing.
    Basic,
    /// Basic spoofing with console.
    BasicWithConsole,
    /// Basic spoofing without webgl.
    BasicNoWebgl,
    /// Mid spoofing.
    Mid,
    /// Full spoofing.
    Full,
    /// No spoofing
    None,
}

impl Tier {
    /// Stealth mode enabled.
    pub fn stealth(&self) -> bool {
        match &self {
            Tier::None => false,
            _ => true,
        }
    }
}

/// The user agent types of profiles we support for stealth.
#[derive(PartialEq, Clone, Copy, Default, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AgentOs {
    #[default]
    /// Linux.
    Linux,
    /// Mac.
    Mac,
    /// Windows.
    Windows,
    /// Android.
    Android,
    /// Unknown.
    Unknown,
}
