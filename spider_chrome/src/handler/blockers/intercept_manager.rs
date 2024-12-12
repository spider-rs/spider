/// Custom network intercept types to expect on a domain
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum NetworkInterceptManager {
    /// tiktok.com
    TikTok,
    /// facebook.com
    Facebook,
    /// amazon.com
    Amazon,
    /// x.com
    X,
    /// LinkedIn,
    LinkedIn,
    /// netflix.com
    Netflix,
    /// medium.com
    Medium,
    /// upwork.com,
    Upwork,
    /// glassdoor.com
    Glassdoor,
    /// ebay.com
    Ebay,
    /// nytimes.com
    Nytimes,
    /// wikipedia.com
    Wikipedia,
    #[default]
    /// Unknown
    Unknown,
}

lazy_static::lazy_static! {
    /// Top tier list of the most common websites visited.
    pub static ref TOP_TIER_LIST: [(&'static str, NetworkInterceptManager); 21] = [
        ("https://www.tiktok.com", NetworkInterceptManager::TikTok),
        ("https://tiktok.com", NetworkInterceptManager::TikTok),
        ("https://www.amazon.", NetworkInterceptManager::Amazon),
        ("https://amazon.", NetworkInterceptManager::Amazon),
        ("https://www.x.com", NetworkInterceptManager::X),
        ("https://x.com", NetworkInterceptManager::X),
        ("https://www.netflix.com", NetworkInterceptManager::Netflix),
        ("https://netflix.com", NetworkInterceptManager::Netflix),
        (
            "https://www.linkedin.com",
            NetworkInterceptManager::LinkedIn
        ),
        ("https://linkedin.com", NetworkInterceptManager::LinkedIn),
        ("https://www.upwork.com", NetworkInterceptManager::Upwork),
        ("https://upwork.com", NetworkInterceptManager::Upwork),
        ("https://www.glassdoor.", NetworkInterceptManager::Glassdoor),
        ("https://glassdoor.", NetworkInterceptManager::Glassdoor),
        ("https://www.medium.com", NetworkInterceptManager::Medium),
        ("https://medium.com", NetworkInterceptManager::Medium),
        ("https://www.ebay.", NetworkInterceptManager::Ebay),
        ("https://ebay.", NetworkInterceptManager::Ebay),
        ("https://www.nytimes.com", NetworkInterceptManager::Nytimes),
        ("https://nytimes.com", NetworkInterceptManager::Nytimes),
        ("wikipedia.org", NetworkInterceptManager::Wikipedia),
    ];
}

/// The find type is own.
#[derive(Default, Debug, Clone, Hash, PartialEq, Eq)]
enum FindType {
    #[default]
    /// Starts with.
    StartsWith,
    /// Contains.
    Contains,
}

impl NetworkInterceptManager {
    /// a custom intercept handle.
    pub fn new(url: &str) -> NetworkInterceptManager {
        TOP_TIER_LIST
            .iter()
            .find(|&(pattern, nm)| {
                if nm.get_pattern() == FindType::StartsWith {
                    url.starts_with(pattern)
                } else {
                    url.contains(pattern)
                }
            })
            .map(|&(_, manager_type)| manager_type)
            .unwrap_or(NetworkInterceptManager::Unknown)
    }
    /// Setup the intercept handle
    pub fn setup(&mut self, url: &str) -> Self {
        NetworkInterceptManager::new(url)
    }

    /// determine the pattern to use.
    fn get_pattern(&self) -> FindType {
        match self {
            NetworkInterceptManager::Wikipedia => FindType::Contains,
            _ => FindType::StartsWith,
        }
    }
}
