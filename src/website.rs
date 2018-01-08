use std::io::Read;
use std::time::Instant;

use scraper::{Html, Selector};
use reqwest;
use colored::*;

/// Represent a website with many links to visit
#[derive(Debug)]
pub struct Website {
    domain: String,
    links: Vec<String>,
    links_visited: Vec<String>,
    pages: Vec<Page>,
}

impl Website {
    /// Initialize Website object with one link
    pub fn new(domain: &str) -> Self {
        // create home link
        let mut links: Vec<String> = Vec::new();
        links.push(format!("{}/", domain));

        Self {
            domain: domain.to_string(),
            links: links,
            links_visited: Vec::new(),
            pages: Vec::new(),
        }
    }

    /// Start to crawling website and continue to crawling while new links found
    pub fn crawl(&mut self) {
        // while we have links, we continue to iteate
        while self.links.len() > 0 {
            let mut new_links: Vec<String> = Vec::new();
            for link in &self.links {
                if self.links_visited.contains(link) {
                    continue;
                }

                let page = Page::new(link, &self.domain);
                for link in page.links.clone() {
                    new_links.push(link);
                }

                self.pages.push(page);
                self.links_visited.push(link.to_string());
            }

            self.links= new_links;
        }
    }

    // Output this website to console
    // pub fn print(&self){
    //     for page in &self.pages {
    //         println!("");
    //         page.print();
    //     }
    // }
}

/// Represent a page of a website
#[derive(Debug)]
pub struct Page {
    url: String,
    /// List of content of h1 tag founded
    h1: Vec<String>,
    /// <title> tag
    title: Option<String>,
    /// <meta name="description" .. > tag 
    meta_description: Option<String>,
    /// <meta name="keywords" .. > tag 
    meta_keywords: Option<String>,
    /// links founded on this page
    links: Vec<String>,
    loaded_time : f64,
}

impl Page {

    /// Launch an HTTP GET query & get all informations
    pub fn new(url: &str, domain : &str) -> Self {
        // fetch HTML & measure time
        let now = Instant::now();
        let html = Self::visit(url);
        let elapsed = now.elapsed();
        let loaded_time = (elapsed.as_secs() as f64) + (elapsed.subsec_nanos() as f64 / 1000_000_000.0);

        let page = Self {
            url: url.to_string(),
            links: Self::get_links(&html, domain),
            loaded_time : loaded_time,
            title: Self::get_title(&html),
            meta_description: Self::get_meta(&html, "description"),
            meta_keywords: Self::get_meta(&html, "keywords"),
            h1: Self::get_h1(&html)
        };

        page.print();

        page
    }


    /// Launch an HTTP GET query to te given URL & parse body response content
    fn visit(url: &str) -> Html {
        let mut res = reqwest::get(url).unwrap();
        let mut body = String::new();
        res.read_to_string(&mut body).unwrap();

        Html::parse_document(&body)
    }


    /// Scrape this page & get some information
    pub fn get_title(html: &Html)-> Option<String>{
        let selector = Selector::parse("title").unwrap();

        for element in html.select(&selector) {
            let title : String =  element.inner_html();

            return Some(title);
        }

        None
    }

    /// Scrape this page & get some information
    pub fn get_meta(html: &Html, name: &str)-> Option<String>{
        let meta_selector = format!("meta[name={}]", name);
        let selector = Selector::parse(&meta_selector).unwrap();

        for element in html.select(&selector) {
            match element.value().attr("content") {
                Some(content) => {
                    if !content.is_empty() {
                        return Some(content.to_string())
                    }
                },
                None => ()
            };
        }

        None
    }


    /// Scrape this page & get some information
    pub fn get_h1(html: &Html)-> Vec<String>{
        let mut h1s: Vec<String> = Vec::new();

        let selector = Selector::parse("h1").unwrap();

        for element in html.select(&selector) {
            let h1 : String =  element.inner_html();
            h1s.push(h1);
        }

        h1s
    }

    /// Parse given page & get all links on it
    fn get_links(html: &Html, domain: &str) -> Vec<String> {
        let mut urls: Vec<String> = Vec::new();

        let selector = Selector::parse("a").unwrap();

        for element in html.select(&selector) {

            match element.value().attr("href") {
                Some(href) => {

                    // Keep only links for this domains
                    match href.find('/') {
                        Some(0) => urls.push(format!("{}{}", domain, href)),
                        Some(_) => (),
                        None => (),
                    };
                }
                None => (),
            };
        }

        urls
    }

    /// 
    pub fn print(&self){
        // DISPLAY URL
        println!("{}", self.url.bold());

        // DISPLAY LOADED TIME
        let loaded_time_output = if self.loaded_time < 0.400 {
            format!("\t- Loaded time: {}", self.loaded_time).green()
        } else if self.loaded_time < 0.800 {

            format!("\t- Loaded time: {}", self.loaded_time).yellow()
        }else {
            format!("\t- Loaded time: {}", self.loaded_time).red()

        };
        println!("{}", loaded_time_output);

        // DISPLAY title
        match &self.title {
            &Some(ref title) => println!("\t{}", format!("- title: {}", title).green()),
            &None => println!("\t{}", "- title: not found".red()),
        }

        // DISPLAY description
        match &self.meta_description {
            &Some(ref description) => println!("\t{}", format!("- description: {}", description).green()),
            &None => println!("\t{}", "- description: not found".red()),
        }

        // DISPLAY description
        match &self.meta_keywords {
            &Some(ref keywords) => println!("\t{}", format!("- Meta keywords: {}", keywords).green()),
            &None => println!("\t{}", "- Meta keywords: not found".red()),
        }

        // DISPLAY H1
        let mut h1_output : String= "\t- h1: ".to_string();
        for h1 in &self.h1 {
            h1_output.push_str(&format!("'{}' ", h1));
        }
        // display in red if no h1 or multiple found
        if self.h1.len() == 1 {
            println!("{}", h1_output.green());
        } else {
            h1_output.push_str("not found");
            println!("{}", h1_output.red());
        }

        // blank line
        println!("");
    }
}


