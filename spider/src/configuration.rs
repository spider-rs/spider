use num_cpus;
use std::env;

/// Structure to configure `Website` crawler
/// <pre>
/// let mut website: Website = Website::new("https://choosealicense.com");
/// website.configuration.blacklist_url.push("https://choosealicense.com/licenses/".to_string());
/// website.configuration.respect_robots_txt = true;
/// website.configuration.verbose = true;
/// localhost.crawl();
/// </pre>
#[derive(Debug, Default)]
pub struct Configuration {
    /// Respect robots.txt file and not scrape not allowed files (not implemented)
    pub respect_robots_txt: bool,
    /// Print page visited on standart output
    pub verbose: bool,
    /// List of pages to not crawl [optional: regex pattern matching]
    pub blacklist_url: Vec<String>,
    /// User-Agent
    pub user_agent: &'static str,
    /// Polite crawling delay in milli seconds
    pub delay: u64,
    /// How many request can be run simultaneously
    pub concurrency: usize,
}

impl Configuration {
    pub fn new() -> Self {
        Self {
            user_agent: concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
            delay: 250,
            concurrency: num_cpus::get() * 4,
            ..Default::default()
        }
    }
}
