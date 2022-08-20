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
//! use spider::packages::robotparser::RobotFileParser;;
//!
//! fn main() {
//!     let parser = RobotFileParser::new("http://www.python.org/robots.txt");
//!     parser.read();
//!     assert!(parser.can_fetch("*", "http://www.python.org/robots.txt"));
//! }
//! ```

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use reqwest::blocking::Client;
use reqwest::blocking::Response;
use reqwest::header::USER_AGENT;
use reqwest::StatusCode;
use std::io::Read;
use url::Url;

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
    /// Url of the website
    url: Url,
    /// Domain url path
    path: String,
    /// Time last checked robots.txt file
    last_checked: i64,
    /// User-agent string
    pub user_agent: String,
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
        let ua = useragent.split('/').nth(0).unwrap_or("").to_lowercase();
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
    pub fn new<T: AsRef<str>>(url: T) -> RobotFileParser {
        let parsed_url = Url::parse(url.as_ref()).unwrap();

        RobotFileParser {
            entries: vec![],
            default_entry: Entry::new(),
            disallow_all: false,
            allow_all: false,
            path: parsed_url.path().to_string(),
            url: parsed_url,
            last_checked: 0i64,
            user_agent: String::from("robotparser-rs"),
        }
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

    /// Sets the URL referring to a robots.txt file.
    pub fn set_url<T: AsRef<str>>(&mut self, url: T) {
        let parsed_url = Url::parse(url.as_ref()).unwrap();
        self.path = parsed_url.path().to_string();
        self.url = parsed_url;
        self.last_checked = 0i64;
    }

    /// Reads the robots.txt URL and feeds it to the parser.
    pub fn read(&mut self, client: &Client) {
        let request = client.get(self.url.as_ref());
        let request = request.header(USER_AGENT, &self.user_agent);
        let mut res = match request.send() {
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
            StatusCode::OK => self.from_response(&mut res),
            _ => {}
        }
    }

    /// Reads the HTTP response and feeds it to the parser.
    pub fn from_response(&mut self, response: &mut Response) {
        let mut buf = String::new();
        response.read_to_string(&mut buf).unwrap();
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

        self.modified();
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
                    .unwrap_or("".to_string());
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
    pub fn can_fetch<T: AsRef<str>>(&self, useragent: T, url: T) -> bool {
        use percent_encoding::percent_decode;

        let useragent = useragent.as_ref();
        let url = url.as_ref();

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
        let decoded_url = String::from_utf8(percent_decode(url.trim().as_bytes()).collect())
            .unwrap_or("".to_string());

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
    pub fn get_crawl_delay<T: AsRef<str>>(&self, useragent: T) -> Option<Duration> {
        let useragent = useragent.as_ref();

        if self.last_checked == 0 {
            return None;
        }

        for entry in &self.entries {
            if entry.applies_to(useragent) {
                return entry.get_crawl_delay();
            }
        }

        let default_entry = &self.default_entry;

        if !default_entry.is_empty() {
            return default_entry.get_crawl_delay();
        }

        None
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
