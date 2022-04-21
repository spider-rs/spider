use std::fs::File;
use std::io::{BufWriter, Write};

pub fn crawl_stub() -> String {
    r#"
    package main

    import (
        "fmt"
    
        "github.com/gocolly/colly/v2"
    )
    
    func main() {
        c := colly.NewCollector()
    
        c.Limit(&colly.LimitRule{
            Delay:      0,
        })

        c.OnHTML(`a[href^="/"]:not([href$=".png"]):not([href$=".jpg"]):not([href$=".mp4"]):not([href$=".mp3"]):not([href$=".gif"]),
        a[href^="https://rsseau.fr"]:not([href$=".png"]):not([href$=".jpg"]):not([href$=".mp4"]):not([href$=".mp3"]):not([href$=".gif"])`, func(e *colly.HTMLElement) {
            e.Request.Visit(e.Attr("href"))
        })
    
        c.OnRequest(func(r *colly.Request) {
            fmt.Println("- visiting ", r.URL)
        })
    
        c.Visit("https://rsseau.fr")
    }
    "#.to_string()
}

pub fn gen_crawl() -> String {
    let crawl_script = String::from("./go-crolly.go");
    let file = File::create(&crawl_script).expect("create go crawl script");
    let mut file = BufWriter::new(file);
    let stub = crawl_stub();
    file.write_all(stub.as_bytes()).expect("Unable to write data");

    crawl_script
}





