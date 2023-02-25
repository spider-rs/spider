//! robots.txt parser for Rust.
//!
//! This package initially started from a fork of <https://docs.rs/robotparser/latest/robotparser/>
//! that has improvements that help our case for speed.
//!
//! The robots.txt Exclusion Protocol is implemented as specified in
//! <http://www.robotstxt.org/norobots-rfc.txt>
//!
//!
//! Add ``extern crate robotparser`` to your crate root and your're good to go!
//!
//! # Examples
//!
//! ```rust,ignore
//! extern crate spider;
//!
//! use spider::packages::robotparser::RobotFileParser;
//! use reqwest::blocking::Client;
//!
//! fn main() {
//!     let parser = RobotFileParser::new();
//!     let client = Client::new();
//!     parser.read(&client, &"http://www.python.org/robots.txt");
//!     assert!(parser.can_fetch("*", "http://www.python.org/robots.txt"));
//! }
//! ```

use compact_str::CompactString;
use reqwest::Client;
use reqwest::Response;
use reqwest::StatusCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A rule line is a single "Allow:" (allowance==True) or "Disallow:"
/// (allowance==False) followed by a path."""
#[derive(Debug, Eq, PartialEq, Clone)]
struct RuleLine {
    /// Path of the rule
    path: String,
    /// Is the rule allowed?
    allowance: bool,
}

#[derive(Debug, Eq, PartialEq, Clone)]
/// Determine the amount of request allowed between navigation or crawls.
pub struct RequestRate {
    /// Amount of request allowed within duration
    pub requests: usize,
    /// Duration in seconds between request
    pub seconds: usize,
}

/// An entry has one or more user-agents and zero or more rulelines
#[derive(Debug, Eq, PartialEq, Clone)]
struct Entry {
    /// Multiple user agents to use
    useragents: Vec<String>,
    /// Rules that should be ignored
    rulelines: Vec<RuleLine>,
    /// Time to wait in between crawls
    crawl_delay: Option<Duration>,
    /// The request rate to respect
    req_rate: Option<RequestRate>,
}

/// robots.txt file parser
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct RobotFileParser {
    /// Entire robots.txt list of urls
    entries: Vec<Entry>,
    /// Base entry to list
    default_entry: Entry,
    /// Dis-allow links reguardless of robots.txt
    disallow_all: bool,
    /// Allow links reguardless of robots.txt
    allow_all: bool,
    /// Time last checked robots.txt file
    last_checked: i64,
}

impl RuleLine {
    fn new(path: &str, allowance: bool) -> RuleLine {
        RuleLine {
            path: path.into(),
            allowance: path == "" && !allowance || allowance,
        }
    }

    fn applies_to(&self, filename: &str) -> bool {
        self.path == "*" || filename.starts_with(&self.path)
    }
}

impl Entry {
    /// Base collection to manage robot.txt data
    fn new() -> Entry {
        Entry {
            useragents: vec![],
            rulelines: vec![],
            crawl_delay: None,
            req_rate: None,
        }
    }

    /// check if this entry applies to the specified agent
    fn applies_to(&self, useragent: &str) -> bool {
        let ua = useragent
            .split('/')
            .nth(0)
            .unwrap_or_default()
            .to_lowercase();
        for agent in &self.useragents {
            if agent == "*" {
                return true;
            }
            if ua.contains(agent) {
                return true;
            }
        }
        false
    }

    /// Preconditions:
    /// - our agent applies to this entry
    /// - filename is URL decoded
    fn allowance(&self, filename: &str) -> bool {
        for line in &self.rulelines {
            if line.applies_to(filename) {
                return line.allowance;
            }
        }
        true
    }

    /// Add to user agent list
    fn push_useragent(&mut self, useragent: &str) {
        self.useragents.push(useragent.to_lowercase());
    }

    /// Add rule to list
    fn push_ruleline(&mut self, ruleline: RuleLine) {
        self.rulelines.push(ruleline);
    }

    /// Determine if user agent exist
    fn has_useragent(&self) -> bool {
        self.useragents.iter().any(|a| a == "*")
    }

    /// Is the user-agent list empty?
    fn is_empty(&self) -> bool {
        self.useragents.is_empty() && self.rulelines.is_empty()
    }

    /// Set the crawl delay for the website
    fn set_crawl_delay(&mut self, delay: Duration) {
        self.crawl_delay = Some(delay);
    }

    /// Determine the crawl delay for the website
    fn get_crawl_delay(&self) -> Option<Duration> {
        self.crawl_delay
    }

    /// Establish request rates between robots.txt crawling sitemaps
    fn set_req_rate(&mut self, req_rate: RequestRate) {
        self.req_rate = Some(req_rate);
    }

    /// Determine the limit allowed between request before being limited.
    fn get_req_rate(&self) -> Option<RequestRate> {
        self.req_rate.clone()
    }
}

impl Default for Entry {
    fn default() -> Entry {
        Entry::new()
    }
}

impl RobotFileParser {
    /// Establish a new robotparser for a website domain
    pub fn new() -> Box<RobotFileParser> {
        RobotFileParser {
            entries: vec![],
            default_entry: Entry::new(),
            disallow_all: false,
            allow_all: false,
            last_checked: 0i64,
        }
        .into()
    }

    /// Returns the time the robots.txt file was last fetched.
    ///
    /// This is useful for long-running web spiders that need to
    /// check for new robots.txt files periodically.
    pub fn mtime(&self) -> i64 {
        self.last_checked
    }

    /// Sets the time the robots.txt file was last fetched to the
    /// current time.
    pub fn modified(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        self.last_checked = now;
    }

    /// Reads the robots.txt URL and feeds it to the parser.
    pub async fn read(&mut self, client: &Client, url: &str) {
        self.modified();

        let request = client.get(&string_concat!(url, "robots.txt"));

        let res = match request.send().await {
            Ok(res) => res,
            Err(_) => {
                return;
            }
        };
        let status = res.status();
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                self.disallow_all = true;
            }
            status
                if status >= StatusCode::BAD_REQUEST
                    && status < StatusCode::INTERNAL_SERVER_ERROR =>
            {
                self.allow_all = true;
            }
            StatusCode::OK => self.from_response(res).await,
            _ => {}
        }
    }

    /// Reads the HTTP response and feeds it to the parser.
    pub async fn from_response(&mut self, response: Response) {
        let buf = response.text().await.unwrap();
        let lines: Vec<&str> = buf.split('\n').collect();
        self.parse(&lines);
    }

    fn _add_entry(&mut self, entry: Entry) {
        if entry.has_useragent() {
            // the default entry is considered last
            if self.default_entry.is_empty() {
                // the first default entry wins
                self.default_entry = entry;
            }
        } else {
            self.entries.push(entry);
        }
    }

    ///
    /// Parse the input lines from a robots.txt file
    ///
    /// We allow that a user-agent: line is not preceded by
    /// one or more blank lines.
    ///
    pub fn parse<T: AsRef<str>>(&mut self, lines: &[T]) {
        use percent_encoding::percent_decode;

        // states:
        //   0: start state
        //   1: saw user-agent line
        //   2: saw an allow or disallow line
        let mut state = 0;
        let mut entry = Entry::new();

        for line in lines {
            let mut ln = line.as_ref();
            if ln.is_empty() {
                match state {
                    1 => {
                        entry = Entry::new();
                        state = 0;
                    }
                    2 => {
                        self._add_entry(entry);
                        entry = Entry::new();
                        state = 0;
                    }
                    _ => {}
                }
            }
            // remove optional comment and strip line
            if let Some(i) = ln.find('#') {
                ln = &ln[0..i];
            }
            ln = ln.trim();
            if ln.is_empty() {
                continue;
            }
            let parts: Vec<&str> = ln.splitn(2, ':').collect();
            if parts.len() == 2 {
                let part0 = parts[0].trim().to_lowercase();
                let part1 = String::from_utf8(percent_decode(parts[1].trim().as_bytes()).collect())
                    .unwrap_or_default();
                match part0 {
                    ref x if x.to_lowercase() == "user-agent" => {
                        if state == 2 {
                            self._add_entry(entry);
                            entry = Entry::new();
                        }
                        entry.push_useragent(&part1);
                        state = 1;
                    }
                    ref x if x.to_lowercase() == "disallow" => {
                        if state != 0 {
                            entry.push_ruleline(RuleLine::new(&part1, false));
                            state = 2;
                        }
                    }
                    ref x if x.to_lowercase() == "allow" => {
                        if state != 0 {
                            entry.push_ruleline(RuleLine::new(&part1, true));
                            state = 2;
                        }
                    }
                    ref x if x.to_lowercase() == "crawl-delay" => {
                        if state != 0 {
                            if let Ok(delay) = part1.parse::<f64>() {
                                let delay_seconds = delay.trunc();
                                let delay_nanoseconds = delay.fract() * 10f64.powi(9);
                                let delay =
                                    Duration::new(delay_seconds as u64, delay_nanoseconds as u32);

                                entry.set_crawl_delay(delay);
                            }
                            state = 2;
                        }
                    }
                    ref x if x.to_lowercase() == "sitemap" => {
                        if state != 0 {
                            state = 2;
                        }
                    }
                    ref x if x.to_lowercase() == "request-rate" => {
                        if state != 0 {
                            let numbers: Vec<Result<usize, _>> =
                                part1.split('/').map(|x| x.parse::<usize>()).collect();
                            if numbers.len() == 2 && numbers[0].is_ok() && numbers[1].is_ok() {
                                let req_rate = RequestRate {
                                    requests: numbers[0].clone().unwrap(),
                                    seconds: numbers[1].clone().unwrap(),
                                };
                                entry.set_req_rate(req_rate);
                            }
                            state = 2;
                        }
                    }
                    _ => {}
                }
            }
        }
        if state == 2 {
            self._add_entry(entry);
        }
    }

    /// Using the parsed robots.txt decide if useragent can fetch url
    pub fn can_fetch<T: AsRef<str>>(&self, useragent: T, url: &str) -> bool {
        use percent_encoding::percent_decode;

        let useragent = useragent.as_ref();

        if self.disallow_all {
            return false;
        }
        if self.allow_all {
            return true;
        }
        // Until the robots.txt file has been read or found not
        // to exist, we must assume that no url is allowable.
        // This prevents false positives when a user erronenously
        // calls can_fetch() before calling read().
        if self.last_checked == 0 {
            return false;
        }
        // search for given user agent matches
        // the first match counts
        let decoded_url =
            String::from_utf8(percent_decode(url.trim().as_bytes()).collect()).unwrap_or_default();

        let url_str = match decoded_url {
            ref u if !u.is_empty() => u,
            _ => "/",
        };

        for entry in &self.entries {
            if entry.applies_to(useragent) {
                return entry.allowance(&url_str);
            }
        }

        // try the default entry last
        let default_entry = &self.default_entry;

        if !default_entry.is_empty() {
            return default_entry.allowance(&url_str);
        }
        // agent not found ==> access granted
        true
    }

    /// Returns the crawl delay for this user agent as a `Duration`, or None if no crawl delay is defined.
    pub fn get_crawl_delay(&self, useragent: &Option<Box<CompactString>>) -> Option<Duration> {
        if self.last_checked == 0 {
            None
        } else {
            let useragent = useragent.as_ref();
            let crawl_delay: Option<Duration> = match useragent {
                Some(ua) => {
                    for entry in &self.entries {
                        if entry.applies_to(ua) {
                            return entry.get_crawl_delay();
                        }
                    }
                    None
                }
                _ => None,
            };

            if crawl_delay.is_some() {
                crawl_delay
            } else {
                let default_entry = &self.default_entry;

                if !default_entry.is_empty() {
                    return default_entry.get_crawl_delay();
                }

                None
            }
        }
    }

    /// Returns the request rate for this user agent as a `RequestRate`, or None if not request rate is defined
    pub fn get_req_rate<T: AsRef<str>>(&self, useragent: T) -> Option<RequestRate> {
        let useragent = useragent.as_ref();
        if self.last_checked == 0 {
            return None;
        }
        for entry in &self.entries {
            if entry.applies_to(useragent) {
                return entry.get_req_rate();
            }
        }
        None
    }
}
