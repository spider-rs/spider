//! `cargo run --example debug`
extern crate env_logger;
extern crate spider;

use env_logger::Env;
use spider::website::Website;

fn main() {
    // enable with RUST_LOG env_logger crate
    let env = Env::default()
        .filter_or("RUST_LOG", "info")
        .write_style_or("RUST_LOG_STYLE", "always");

    env_logger::init_from_env(env);

    let mut website: Website = Website::new("https://rsseau.fr");
    website.configuration.respect_robots_txt = true;
    website.configuration.delay = 15; // Defaults to 250 ms
    website.configuration.user_agent = "SpiderBot".into(); // Defaults to spider/x.y.z, where x.y.z is the library version

    website.crawl();
}
