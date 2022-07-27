//! `cargo run --example example`
extern crate spider;

use spider::website::Website;

fn main() {
  let mut website: Website = Website::new("https://rsseau.fr");
  website.configuration.blacklist_url.push("https://rsseau.fr/resume".to_string());
  website.configuration.respect_robots_txt = true;
  website.configuration.subdomains = false;
  website.configuration.delay = 15; // Defaults to 250 ms
  website.configuration.concurrency = 10; // Defaults to number of cpus available
  website.configuration.user_agent = "SpiderBot".into(); // Defaults to spider/x.y.z, where x.y.z is the library version
  website.crawl();

  for page in website.get_pages() {
    println!("- {}", page.get_url());
  }
}

