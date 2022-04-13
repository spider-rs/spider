use spider::website::Website;

fn main() {
    let mut website: Website = Website::new("https://cmoran.xyz");
    // website.configuration.add_blacklist_url("https://cmoran.xyz/writing");
    website.configuration.add_blacklist_pattern(".png");
    website.configuration.add_blacklist_pattern(".webp");
    website.configuration.add_blacklist_pattern(".gz");
    website.configuration.add_blacklist_pattern(".JPG");
    website.configuration.respect_robots_txt = false;
    website.configuration.verbose = true; // Defaults to false
    website.configuration.delay = 1; // Defaults to 250 ms
    website.crawl();
}
