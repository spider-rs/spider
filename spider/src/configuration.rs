use num_cpus;

/// Structure to configure `Website` crawler
/// <pre>
/// let mut website: Website = Website::new("https://choosealicense.com");
/// website.configuration.blacklist_url.push("https://choosealicense.com/licenses/".to_string());
/// website.configuration.respect_robots_txt = true;
/// website.configuration.verbose = true;
/// localhost.crawl();
/// </pre>
#[derive(Default, Debug)]
pub struct Configuration {
    /// Respect robots.txt file and not scrape not allowed files (not implemented)
    pub respect_robots_txt: bool,
    /// Print page visited on standart output
    pub verbose: bool,
    /// List of pages to not crawl
    pub blacklist_url: Vec<String>,
    /// Blacklist any link containing these patterns
    pub blacklist_pattern: Vec<String>,
    /// User-Agent
    pub user_agent: &'static str,
    /// Polite crawling delay in milli seconds
    pub delay: u64,
    /// How many request can be run simultaneously
    pub concurrency: usize,
}

impl Configuration {
    /// Create default Spider Configuration
    pub fn new() -> Self {
        Self {
            user_agent: "spider/1.5.0",
            delay: 250,
            concurrency: num_cpus::get() * 4,
            ..Default::default()
        }
    }

    /// Add pattern, for which any link containing it will be blacklisted
    pub fn add_blacklist_pattern(&mut self, pattern: impl Into<String>) {
        self.blacklist_pattern.push(pattern.into());
    }

    /// Add specific url to blacklist
    pub fn add_blacklist_url(&mut self, url: impl Into<String>) {
        self.blacklist_url.push(url.into());
    }
}
