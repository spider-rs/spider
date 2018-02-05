extern crate reqwest;
extern crate scraper;

mod website;
mod page;

use website::Website;

fn main() {
    // let mut localhost = Website::new("http://rousseau-alexandre.fr");
    let mut localhost = Website::new("http://localhost:4000");
    localhost.crawl();

    for page in localhost.get_pages() {
        println!("- {}", page.get_url());
    }
}
