use num_cpus;

/// Structure to configure `Website` crawler
/// ```rust
/// use spider::website::Website;
/// let mut website: Website = Website::new("https://choosealicense.com");
/// website.configuration.blacklist_url.push("https://choosealicense.com/licenses/".to_string());
/// website.configuration.respect_robots_txt = true;
/// website.configuration.subdomains = true;
/// website.configuration.tld = true;
/// website.crawl();
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
    pub blacklist_url: Vec<String>,
    /// User-Agent
    pub user_agent: String,
    /// Polite crawling delay in milli seconds.
    pub delay: u64,
    /// How many request can be run simultaneously.
    pub concurrency: usize,
}

/// get the user agent from the top agent list randomly.
#[cfg(any(feature = "ua_generator"))]
pub fn get_ua() -> String {
    ua_generator::ua::spoof_ua().into()
}

/// get the user agent via cargo package + version.
#[cfg(not(any(feature = "ua_generator")))]
pub fn get_ua() -> String {
    use std::env;

    format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

impl Configuration {
    /// Represents crawl configuration for a website.
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
            delay: 250,
            concurrency,
            ..Default::default()
        }
    }
}
