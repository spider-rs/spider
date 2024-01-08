use std::fs::File;
use std::io::{BufWriter, Write};

pub fn crawl_stub() -> String {
    r#"

    const Crawler = require("crawler");

    const base = "https://rsseau.fr";
    const crawledPages = { [base]: true };
    const ignoreSelector = `:not([href$=".png"]):not([href$=".jpg"]):not([href$=".mp4"]):not([href$=".mp3"]):not([href$=".gif"])`;
    
    const crawlOptions = {
      skipEventRequest: false,
    };
    
    const callback = (error, res) => {
      if (error) {
        console.error(error);
      } else {
        const $ = res.$;
    
        $(`a[href^="/"]${ignoreSelector},a[href^="${base}"]${ignoreSelector}`).each(
          (_i, elem) => {
            if (!crawledPages[elem.attribs.href]) {
              crawledPages[elem.attribs.href] = true;
              directCrawl(`${base}${elem.attribs.href}`);
            }
          }
        );
      }
    };
    
    const crawler = new Crawler({
      maxConnections: 10,
      rateLimit: 0,
      callback,
    });
    
    const directCrawl = (uri) => {
      crawler.direct({
        uri,
        callback,
        ...crawlOptions,
      });
    };
    
    directCrawl(base);    
  
    "#.to_string()
}

pub fn gen_crawl() -> String {
    let crawl_script = String::from("./node-crawler.js");
    let file = File::create(&crawl_script).expect("create js crawl script");
    let mut file = BufWriter::new(file);
    let stub = crawl_stub();

    file.write_all(stub.as_bytes())
        .expect("Unable to write data");

    crawl_script
}
