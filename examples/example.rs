extern crate spider;

use spider::website::Website;

fn main() {
  let mut website: Website = Website::new("https://rsseau.fr");
  website.configuration.blacklist_url.push("https://rsseau.fr/resume".to_string());
  website.configuration.respect_robots_txt = true;
  website.configuration.delay = 2000; // Defaults to 250 ms
  website.configuration.concurrency = 10; // Defaults to number of cpus available
  website.configuration.user_agent = "myapp/version"; // Defaults to spider/x.y.z, where x.y.z is the library version
  website.configure_robots_parser(); // Defaults to extracting robot configuration at 'website.crawl' call if respect_robots_txt=true
  website.crawl();
}

