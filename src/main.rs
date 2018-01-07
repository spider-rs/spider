extern crate reqwest;
extern crate scraper;

mod website;


use website::Website;


fn main() {
    let mut localhost = Website::new("http://localhost:4000");
    localhost.crawl();

    println!("{:?}", localhost);
}
