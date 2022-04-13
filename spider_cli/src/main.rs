extern crate spider;

use spider::website::Website;
use std::collections::HashMap;

fn parse_args(mut args: impl Iterator<Item = String>) -> HashMap<String, String> {
    let mut flags = HashMap::new();

    while let Some(arg) = args.next() {
        if let Some(flag) = arg.strip_prefix('-') {
            if let Some(option) = flag.strip_prefix('-') {
                flags.insert(option.into(), args.next().unwrap_or_default());
            } else {
                for fchar in flag.chars() {
                    flags.insert(fchar.into(), String::from("1"));
                }
            }
        }
    }

    flags
}

fn main() {
    let options = parse_args(std::env::args());
    let mut website: Website = Website::new(&options["domain"]);

    if options.contains_key("respect_robots_txt") {
        website.configuration.respect_robots_txt = options["respect_robots_txt"] == "true";
    }
    if options.contains_key("verbose") {
        website.configuration.verbose = options["verbose"] == "true";
    }
    if options.contains_key("delay") {
        website.configuration.delay = options["delay"].parse::<u64>().unwrap();
    }
    if options.contains_key("concurrency") {
        website.configuration.concurrency = options["concurrency"].parse::<usize>().unwrap();
    }
    if options.contains_key("blacklist_url") {
        website
            .configuration
            .blacklist_url
            .push(options["blacklist_url"].to_string());
    }

    if options.contains_key("user_agent") {
        website.configuration.user_agent =
            Box::leak(options["user_agent"].to_owned().into_boxed_str());
    }

    // TODO: add on_link_find_callback eval function
    // if options.contains_key("on_link_find_callback") {
    //     website.on_link_find_callback = options["on_link_find_callback"];
    // }

    website.crawl();
}
