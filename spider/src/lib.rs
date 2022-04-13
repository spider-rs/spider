extern crate num_cpus;
extern crate rayon;
extern crate reqwest;
extern crate robotparser_fork;
extern crate scraper;
extern crate tokio;
extern crate url;

/// Configuration structure for `Website`
pub mod configuration;
/// A page scraped
pub mod page;
/// Application utils
pub mod utils;
/// A website to crawl
pub mod website;
