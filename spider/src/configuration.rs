use std::time::Duration;

use compact_str::CompactString;

/// Structure to configure `Website` crawler
/// ```rust
/// use spider::website::Website;
/// let mut website: Website = Website::new("https://choosealicense.com");
/// website.configuration.blacklist_url.insert(Default::default()).push("https://choosealicense.com/licenses/".to_string().into());
/// website.configuration.respect_robots_txt = true;
/// website.configuration.subdomains = true;
/// website.configuration.tld = true;
/// ```
#[derive(Debug, Default, Clone)]
pub struct Configuration {
    /// Respect robots.txt file and not scrape not allowed files.
    pub respect_robots_txt: bool,
    /// Allow sub-domains.
    pub subdomains: bool,
    /// Allow all tlds for domain.
    pub tld: bool,
    /// List of pages to not crawl. [optional: regex pattern matching]
    pub blacklist_url: Option<Box<Vec<CompactString>>>,
    /// User-Agent
    pub user_agent: Option<Box<CompactString>>,
    /// Polite crawling delay in milli seconds.
    pub delay: u64,
    /// Crawl channel buffer tuned to callback.
    pub channel_buffer: i32,
    /// Request max timeout per page
    pub request_timeout: Option<Box<Duration>>,
    /// Use HTTP2 for connection. Enable if you know the website has http2 support.
    pub http2_prior_knowledge: bool,
    /// Use proxy list for performing network request.
    pub proxies: Option<Box<Vec<String>>>,
}

/// get the user agent from the top agent list randomly.
#[cfg(any(feature = "ua_generator"))]
pub fn get_ua() -> &'static str {
    ua_generator::ua::spoof_ua()
}

/// get the user agent via cargo package + version.
#[cfg(not(any(feature = "ua_generator")))]
pub fn get_ua() -> &'static str {
    use std::env;

    lazy_static! {
        static ref AGENT: &'static str =
            concat!(env!("CARGO_PKG_NAME"), '/', env!("CARGO_PKG_VERSION"));
    };

    AGENT.as_ref()
}

impl Configuration {
    /// Represents crawl configuration for a website.
    pub fn new() -> Self {
        Self {
            delay: 0,
            channel_buffer: 111,
            request_timeout: Some(Box::new(Duration::from_millis(15000))),
            ..Default::default()
        }
    }

    #[cfg(feature = "regex")]
    /// compile the regex for the blacklist
    pub fn get_blacklist(&self) -> Box<Vec<regex::Regex>> {
        match &self.blacklist_url {
            Some(blacklist) => {
                let blacklist = blacklist
                    .iter()
                    .filter_map(|pattern| match regex::Regex::new(&pattern) {
                        Ok(re) => Some(re),
                        _ => None,
                    })
                    .collect::<Vec<regex::Regex>>();

                Box::new(blacklist)
            }
            _ => Default::default(),
        }
    }

    #[cfg(not(feature = "regex"))]
    /// handle the blacklist options
    pub fn get_blacklist(&self) -> Box<Vec<CompactString>> {
        match &self.blacklist_url {
            Some(blacklist) => blacklist.to_owned(),
            _ => Default::default(),
        }
    }
}
