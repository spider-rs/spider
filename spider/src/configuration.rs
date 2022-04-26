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
        let logical_cpus = num_cpus::get();
        let physical_cpus = num_cpus::get_physical();

        // determine simultaneous multithreading
        let concurrency = if logical_cpus > physical_cpus {
            logical_cpus / physical_cpus
        } else {
            logical_cpus
        } * 4;
        
        Self {
            user_agent: concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
            delay: 250,
            concurrency,
            ..Default::default()
        }
    }
}
