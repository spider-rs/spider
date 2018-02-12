/// Structure to configure `Website` crawler
#[derive(Debug)]
pub struct Configuration {
    /// time to wait between all queries (not implemented)
    pub polite_delay: i8,
    /// Respect robot.txt file and not scrape not allowed files (not implemented)
    pub respect_robot_txt: bool,
    /// Print page visited on standart output
    pub verbose: bool,
    /// List of page to not crawl
    pub blacklist_url: Vec<String>,
}

impl Configuration {
    pub fn new() -> Self {
        Self {
            polite_delay: 0,
            respect_robot_txt: false,
            verbose: false,
            blacklist_url: Vec::new(),
        }
    }
}