extern crate reqwest;
extern crate scraper;

mod website;
mod page;

use std::env;
use website::Website;

fn help() {
	println!("usage:\r\nspider <string>\r\nDisplay all URLs containing in a domain.")
}

fn main() {
	let args: Vec<String> = env::args().collect();

	match args.len() {
		2 => {
			let domain : String = args[1].to_string();
		    let mut localhost = Website::new(&domain);
		    localhost.crawl();

		    for page in localhost.get_pages() {
		        println!("- {}", page.get_url());
		    }
		},
		_ =>  help(),

	}
}
