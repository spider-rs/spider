/// Structure to configure `Website` crawler
#[derive(Debug)]
pub struct Configuration {
    /// Respect robots.txt file and not scrape not allowed files (not implemented)
    pub respect_robots_txt: bool,
    /// Print page visited on standart output
    pub verbose: bool,
    /// List of page to not crawl
    pub blacklist_url: Vec<String>,
}

impl Configuration {
    pub fn new() -> Self {
        Self {
            respect_robots_txt: false,
            verbose: false,
            blacklist_url: Vec::new(),
        }
    }
}