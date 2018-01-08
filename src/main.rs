extern crate reqwest;
extern crate scraper;
extern crate colored;



mod website;


use website::Website;


fn main() {
    let mut localhost = Website::new("http://rousseau-alexandre.fr");
    localhost.crawl();
    localhost.crawl();
    localhost.crawl();

    localhost.print();
}
