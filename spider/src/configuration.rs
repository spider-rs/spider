use num_cpus;
use std::env;

/// Structure to configure `Website` crawler
/// ```rust
/// use spider::website::Website;
/// let mut website: Website = Website::new("https://choosealicense.com");
/// website.configuration.blacklist_url.push("https://choosealicense.com/licenses/".to_string());
/// website.configuration.respect_robots_txt = true;
/// website.crawl();
/// ```
#[derive(Debug, Default)]
pub struct Configuration {
    /// Respect robots.txt file and not scrape not allowed files.
    pub respect_robots_txt: bool,
    /// List of pages to not crawl. [optional: regex pattern matching]
    pub blacklist_url: Vec<String>,
    /// User-Agent
    pub user_agent: String,
    /// Polite crawling delay in milli seconds.
    pub delay: u64,
    /// How many request can be run simultaneously.
    pub concurrency: usize,
    /// Attempt random spoof UA across top 4 agents.
    pub random_spoof_ua: bool,
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
            user_agent: concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")).into(),
            delay: 250,
            concurrency,
            ..Default::default()
        }
    }
    /// Get a random UA from the most popular list auto generated from [https://techblog.willshouse.com/2012/01/03/most-common-user-agents/].
    pub fn spoof_ua(&mut self) -> String {
        let moz_base = "Mozilla/5.0";
        let agent_xshared = format!("{} (Windows NT 10.0; Win64; x64", moz_base);
        let chrome_version = |v| {
            format!("{}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{} Safari/537.36", agent_xshared, v)
        };
        let chrome_100 = "100.0.4896.127";
        let chrome_101 = "101.0.4951.54";
        let firefox_100 = format!("{}; rv:100.0) Gecko/20100101 Firefox/100.0", agent_xshared);
        let safari_100 = format!("{} (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.4 Safari/605.1.15", moz_base);
        let agent_list = vec![chrome_version(chrome_100), chrome_version(chrome_101), firefox_100, safari_100]; // list of user agents to choose from.

        let mut agent = 0;

        extern "C" {
            /// C seed random.
            fn srand() -> u32;
            /// return random from seed.
            fn rand() -> u32;
        }
        
        // basic roll dice randomize.
        unsafe {
            srand();
            let random = rand().to_string();
            let random = random.chars().rev().nth(0).unwrap();
            let random: u32 = random.to_digit(10).unwrap();

            if random > 8 {
                agent = 3;
            } else if random > 6 {
                agent = 2;
            } else if random > 3 {
                agent = 1;
            };
        }

        let agent = agent_list[agent].to_owned();
        self.user_agent = agent.clone();

        agent
    }
}


#[test]
fn spoof_ua() {
    let mut config = Configuration::new();
    let ua = config.spoof_ua();
    
    assert_eq!(
        ua,
        config.user_agent
    );
}