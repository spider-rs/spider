use page::Page;


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

    /// Start to crawl website
    pub fn crawl(&mut self) {
        // scrawl while links exists
        while self.links.len() > 0 {
            let mut new_links: Vec<String> = Vec::new();
            for link in &self.links {
                // verify that URL was not already scrawled
                if self.links_visited.contains(link) {
                    continue;
                }

                // scrape page & found links
                let page = Page::new(link);
                for link_founded in page.links(&self.domain) {
                    // add only links not already vistited
                    if !self.links_visited.contains(&link_founded) {
                        new_links.push(link_founded);
                    }
                }
                // add page to scrawled pages

                self.pages.push(page);
                self.links_visited.push(link.to_string());
            }

            self.links = new_links.clone();
        }

    }

    // ///
    // pub fn print(&self) {
    //     // DISPLAY URL
    //     println!("{}", self.url.bold());

    //     // DISPLAY LOADED TIME
    //     let loaded_time_output = if self.loaded_time < 0.400 {
    //         format!("\t- Loaded time: {}", self.loaded_time).green()
    //     } else if self.loaded_time < 0.800 {

    //         format!("\t- Loaded time: {}", self.loaded_time).yellow()
    //     } else {
    //         format!("\t- Loaded time: {}", self.loaded_time).red()

    //     };
    //     println!("{}", loaded_time_output);

    //     // DISPLAY title
    //     match &self.title {
    //         &Some(ref title) => println!("\t{}", format!("- title: {}", title).green()),
    //         &None => println!("\t{}", "- title: not found".red()),
    //     }

    //     // DISPLAY description
    //     match &self.meta_description {
    //         &Some(ref description) => {
    //             println!("\t{}", format!("- description: {}", description).green())
    //         }
    //         &None => println!("\t{}", "- description: not found".red()),
    //     }

    //     // DISPLAY description
    //     match &self.meta_keywords {
    //         &Some(ref keywords) => {
    //             println!("\t{}", format!("- Meta keywords: {}", keywords).green())
    //         }
    //         &None => println!("\t{}", "- Meta keywords: not found".red()),
    //     }

    //     // DISPLAY H1
    //     let mut h1_output: String = "\t- h1: ".to_string();
    //     for h1 in &self.h1 {
    //         h1_output.push_str(&format!("'{}' ", h1));
    //     }
    //     // display in red if no h1 or multiple found
    //     if self.h1.len() == 1 {
    //         println!("{}", h1_output.green());
    //     } else {
    //         h1_output.push_str("not found");
    //         println!("{}", h1_output.red());
    //     }

    //     // blank line
    //     println!("");
    // }
}
