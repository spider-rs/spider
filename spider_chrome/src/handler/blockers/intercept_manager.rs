use phf::phf_map;

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
    /// tcgplayer.com
    Tcgplayer,
    #[default]
    /// Unknown
    Unknown,
}

/// Top tier 100 domain list.
static DOMAIN_MAP: phf::Map<&'static str, NetworkInterceptManager> = phf_map! {
    "tiktok.com" => NetworkInterceptManager::TikTok,
    "facebook.com" => NetworkInterceptManager::Facebook,
    "amazon.com" => NetworkInterceptManager::Amazon,
    "x.com" => NetworkInterceptManager::X,
    "linkedin.com" => NetworkInterceptManager::LinkedIn,
    "netflix.com" => NetworkInterceptManager::Netflix,
    "medium.com" => NetworkInterceptManager::Medium,
    "upwork.com" => NetworkInterceptManager::Upwork,
    "glassdoor.com" => NetworkInterceptManager::Glassdoor,
    "ebay.com" => NetworkInterceptManager::Ebay,
    "nytimes.com" => NetworkInterceptManager::Nytimes,
    "wikipedia.org" => NetworkInterceptManager::Wikipedia,
    "tcgplayer.com" => NetworkInterceptManager::Tcgplayer,
};

impl NetworkInterceptManager {
    pub fn new(url: &Option<Box<url::Url>>) -> NetworkInterceptManager {
        if let Some(parsed_url) = url {
            if let Some(domain) = parsed_url.domain() {
                // list of top websites should at most two - can always do a second pass.
                let domain_parts: Vec<&str> = domain.split('.').collect();

                let base_domain = if domain_parts.len() > 2 {
                    format!(
                        "{}.{}",
                        domain_parts[domain_parts.len() - 2],
                        domain_parts[domain_parts.len() - 1]
                    )
                } else {
                    domain.to_string()
                };

                return *DOMAIN_MAP
                    .get(&base_domain)
                    .unwrap_or(&NetworkInterceptManager::Unknown);
            }
        }
        NetworkInterceptManager::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    /// Helper function to create an Option<Box<Url>> from a string
    fn create_url(url: &str) -> Option<Box<Url>> {
        Url::parse(url).ok().map(Box::new)
    }

    #[test]
    fn test_known_domains() {
        let cases = vec![
            ("http://www.tiktok.com", NetworkInterceptManager::TikTok),
            ("https://facebook.com", NetworkInterceptManager::Facebook),
            ("https://www.amazon.com", NetworkInterceptManager::Amazon),
            ("https://subdomain.x.com", NetworkInterceptManager::X),
            (
                "https://linkedin.com/in/someone",
                NetworkInterceptManager::LinkedIn,
            ),
            (
                "https://www.netflix.com/browse",
                NetworkInterceptManager::Netflix,
            ),
            ("https://medium.com", NetworkInterceptManager::Medium),
            ("https://sub.upwork.com", NetworkInterceptManager::Upwork),
            ("https://glassdoor.com", NetworkInterceptManager::Glassdoor),
            ("https://ebay.com", NetworkInterceptManager::Ebay),
            (
                "https://nytimes.com/section/world",
                NetworkInterceptManager::Nytimes,
            ),
            (
                "https://en.wikipedia.org/wiki/Rust",
                NetworkInterceptManager::Wikipedia,
            ),
            (
                "https://market.tcgplayer.com",
                NetworkInterceptManager::Tcgplayer,
            ),
        ];

        for (url, expected) in cases {
            assert_eq!(NetworkInterceptManager::new(&create_url(url)), expected);
        }
    }

    #[test]
    fn test_unknown_domains() {
        let cases = vec![
            "https://www.unknown.com",
            "http://subdomain.randomstuff.org",
            "https://notindatabase.co.uk",
            "https://another.unknown.site",
        ];

        for url in cases {
            assert_eq!(
                NetworkInterceptManager::new(&create_url(url)),
                NetworkInterceptManager::Unknown
            );
        }
    }

    #[test]
    fn test_invalid_urls() {
        let cases = vec!["not-a-url", "ftp://invalid.protocol.com", "http://", ""];

        for url in cases {
            assert_eq!(
                NetworkInterceptManager::new(&create_url(url)),
                NetworkInterceptManager::Unknown
            );
        }
    }
}
