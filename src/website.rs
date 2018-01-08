use std::io::Read;

use scraper::{Html, Selector};
use reqwest;

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

    /// Start to crawling website
    /// @todo iterate while links exists
    pub fn crawl(&mut self) {
        let mut new_links: Vec<String> = Vec::new();

        for link in &self.links {

            if self.links_visited.contains(link) {
                continue;
            }

            let page = Page::new(link, &self.domain);
            let mut links_founded = page.links.clone();

            new_links.append(&mut links_founded);

            self.pages.push(page);

            self.links_visited.push(link.to_string());
        }

        self.links.append(&mut new_links);
    }
}

/// Represent a page of a website
#[derive(Debug)]
pub struct Page {
    url: String,
    h1 : Vec<String>,
    links: Vec<String>,
}

impl Page {

    /// Launch an HTTP GET query & get all informations
    pub fn new(url: &str, domain : &str) -> Self {
        println!("[x] Fetch {}", url);

        let html = Self::visit(url);

        let links: Vec<String> = Self::get_links(&html, domain);
        let h1: Vec<String> = Self::get_h1(&html);

        Self {
            url: url.to_string(),
            links: links,
            h1: h1
        }
    }


    /// Launch an HTTP GET query to te given URL & parse body response content
    fn visit(url: &str) -> Html {
        let mut res = reqwest::get(url).unwrap();
        let mut body = String::new();
        res.read_to_string(&mut body).unwrap();

        Html::parse_document(&body)
    }


    /// Scrape this page & get some information
    pub fn get_h1(html: &Html)-> Vec<String>{
        let mut h1s: Vec<String> = Vec::new();

        let selector = Selector::parse("h1").unwrap();

        for element in html.select(&selector) {
            let h1 : String =  element.value().name().to_string();
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
}


