extern crate reqwest;
extern crate scraper;
extern crate colored;



mod website;


use website::Website;


fn main() {
    // let mut localhost = Website::new("http://rousseau-alexandre.fr");
    let mut localhost = Website::new("http://localhost:4000");
    localhost.crawl();

    localhost.print();
}
