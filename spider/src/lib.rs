extern crate num_cpus;
extern crate rayon;
extern crate reqwest;
extern crate robotparser_fork;
extern crate scraper;
extern crate tokio;
extern crate url;
extern crate hashbrown;
extern crate log;
#[macro_use]
extern crate lazy_static;

/// Configuration structure for `Website`.
pub mod configuration;
/// A page scraped.
pub mod page;
/// Application utils.
pub mod utils;
/// A website to crawl.
pub mod website;

#[cfg(feature = "regex")]
/// Black list checking url exist with Regex.
pub mod black_list {
    use regex::Regex;
    /// check if link exist in blacklists with regex.
    pub fn contains(blacklist_url: &Vec<String>, link: &String) -> bool {
        for pattern in blacklist_url {
            let re = Regex::new(pattern).unwrap();
            if re.is_match(link) {
                return true;
            }
        }

        return false;
    }
}

#[cfg(not(feature = "regex"))]
/// Black list checking url exist.
pub mod black_list {
    /// check if link exist in blacklists.
    pub fn contains(blacklist_url: &Vec<String>, link: &String) -> bool {
        blacklist_url.contains(&link)
    }
}
