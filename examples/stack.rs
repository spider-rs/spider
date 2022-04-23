extern crate spider;

use spider::website::Website;

/// example using the stack to crawl the website until no more links are found
fn main() {
  let mut website: Website = Website::new("https://rsseau.fr");
  website.configuration.blacklist_url.push("https://rsseau.fr/resume".to_string());
  website.configuration.respect_robots_txt = true;
  website.crawl_stack(None);
}

