include!(concat!(env!("OUT_DIR"), "/domain_map.rs"));

impl NetworkInterceptManager {
    pub fn new(url: &Option<Box<url::Url>>) -> NetworkInterceptManager {
        if let Some(parsed_url) = url {
            if let Some(domain) = parsed_url.domain() {
                let mut domain_parts: Vec<&str> = domain.split('.').collect();

                let base = DOMAIN_MAP.get(if domain_parts.len() >= 2 {
                    domain_parts[domain_parts.len() - 2]
                } else {
                    domain
                });
                let base = if base.is_none() && domain_parts.len() >= 3 {
                    domain_parts.pop();
                    DOMAIN_MAP.get(&domain_parts.join("."))
                } else {
                    base
                };

                return *base.unwrap_or(&NetworkInterceptManager::UNKNOWN);
            }
        }
        NetworkInterceptManager::UNKNOWN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    // Helper function to create an Option<Box<Url>> from a string
    fn create_url(url: &str) -> Option<Box<Url>> {
        Url::parse(url).ok().map(Box::new)
    }

    #[test]
    fn test_known_domains() {
        let cases = vec![
            ("http://www.tiktok.com", NetworkInterceptManager::TIKTOK),
            ("https://facebook.com", NetworkInterceptManager::FACEBOOK),
            ("https://www.amazon.com", NetworkInterceptManager::AMAZON),
            ("https://subdomain.x.com", NetworkInterceptManager::X),
            (
                "https://linkedin.com/in/someone",
                NetworkInterceptManager::LINKEDIN,
            ),
            (
                "https://www.netflix.com/browse",
                NetworkInterceptManager::NETFLIX,
            ),
            ("https://medium.com", NetworkInterceptManager::MEDIUM),
            ("https://sub.upwork.com", NetworkInterceptManager::UPWORK),
            ("https://glassdoor.com", NetworkInterceptManager::GLASSDOOR),
            ("https://ebay.com", NetworkInterceptManager::EBAY),
            (
                "https://nytimes.com/section/world",
                NetworkInterceptManager::NYTIMES,
            ),
            (
                "https://en.wikipedia.org/wiki/Rust",
                NetworkInterceptManager::WIKIPEDIA,
            ),
            (
                "https://market.tcgplayer.com",
                NetworkInterceptManager::TCGPLAYER,
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
                NetworkInterceptManager::UNKNOWN
            );
        }
    }

    #[test]
    fn test_invalid_urls() {
        let cases = vec!["not-a-url", "ftp://invalid.protocol.com", "http://", ""];

        for url in cases {
            assert_eq!(
                NetworkInterceptManager::new(&create_url(url)),
                NetworkInterceptManager::UNKNOWN
            );
        }
    }
}
