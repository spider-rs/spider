extern crate spider;
use crate::spider::website::Website;

fn main() {
    let mut website: Website = Website::new("https://rsseau.fr");
    website.configuration.blacklist_url.push("https://rsseau.fr/resume".to_string());
    website.configuration.respect_robots_txt = true;
    website.configuration.verbose = true;
    website.configure_robots_parser();
    website.crawl();
}

