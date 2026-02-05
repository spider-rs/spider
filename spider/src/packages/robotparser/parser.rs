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

use crate::compact_str::CompactString;
use crate::Client;
#[cfg(feature = "regex")]
use hashbrown::HashSet;
#[cfg(feature = "regex")]
use regex::RegexSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A rule line is a single "Allow:" (allowance==True) or "Disallow:"
/// (allowance==False) followed by a path."""
#[derive(Debug, Eq, PartialEq, Clone)]
#[cfg(not(feature = "regex"))]
pub struct RuleLine {
    /// Path of the rule
    pub path: String,
    /// Is the rule allowed?
    pub allowance: bool,
}

/// A rule line is a single "Allow:" (allowance==True) or "Disallow:"
/// (allowance==False) followed by a path."""
#[derive(Debug, Clone)]
#[cfg(feature = "regex")]
pub struct RuleLine {
    /// Path of the rule
    pub path: Option<regex::Regex>,
    /// Is the rule allowed?
    pub allowance: bool,
}

#[derive(Debug, Eq, PartialEq, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Determine the amount of request allowed between navigation or crawls.
pub struct RequestRate {
    /// Amount of request allowed within duration
    pub requests: usize,
    /// Duration in seconds between request
    pub seconds: usize,
}

/// An entry has one or more user-agents and zero or more rulelines
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "regex"), derive(Eq, PartialEq))]
pub struct Entry {
    /// Multiple user agents to use
    pub useragents: Vec<String>,
    /// Rules that should be ignored
    pub rulelines: Vec<RuleLine>,
    /// Time to wait in between crawls
    pub crawl_delay: Option<Duration>,
    /// The request rate to respect
    pub req_rate: Option<RequestRate>,
}

/// robots.txt file parser
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "regex"), derive(Eq, PartialEq))]
pub struct RobotFileParser {
    /// Entire robots.txt list of urls
    entries: Vec<Entry>,
    /// Base entry to list
    default_entry: Entry,
    /// Dis-allow links reguardless of robots.txt
    pub disallow_all: bool,
    /// Allow links reguardless of robots.txt
    pub allow_all: bool,
    /// Time last checked robots.txt file
    pub last_checked: i64,
    /// Disallow list of regex paths to ignore.
    #[cfg(feature = "regex")]
    pub disallow_paths_regex: RegexSet,
    /// Disallow list of paths to ignore.
    #[cfg(feature = "regex")]
    pub disallow_paths: HashSet<String>,
    /// Disallow list of regex agents to ignore.
    #[cfg(feature = "regex")]
    pub disallow_agents_regex: RegexSet,
    /// Wild card agent provided.
    #[cfg(feature = "regex")]
    pub wild_card_agent: bool,
    /// Disallow list of agents to ignore.
    #[cfg(feature = "regex")]
    pub disallow_agents: HashSet<String>,
}

impl RuleLine {
    #[cfg(feature = "regex")]
    fn new(path: &str, allowance: bool) -> RuleLine {
        use regex::Regex;

        RuleLine {
            path: match Regex::new(path) {
                Ok(r) => Some(r),
                _ => None,
            },
            allowance: path.is_empty() && !allowance || allowance,
        }
    }

    #[cfg(not(feature = "regex"))]
    fn new(path: &str, allowance: bool) -> RuleLine {
        RuleLine {
            path: path.into(),
            allowance: path.is_empty() && !allowance || allowance,
        }
    }

    #[cfg(not(feature = "regex"))]
    fn applies_to(&self, pathname: &str) -> bool {
        if self.path == "*"
            || self.path == "/" && pathname == "/"
            || self.path.ends_with("/") && pathname.starts_with(&self.path)
        {
            true
        } else {
            self.path
                .strip_suffix('*')
                .map_or(false, |prefix| pathname.starts_with(prefix))
                || pathname == self.path
        }
    }

    #[cfg(feature = "regex")]
    fn applies_to(&self, pathname: &str) -> bool {
        match self.path {
            Some(ref regex) => regex.is_match(pathname),
            _ => false,
        }
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

    /// Prepare the user-agent string: strip version suffix, lowercase once.
    #[inline]
    fn prepare_useragent(useragent: &str) -> String {
        useragent
            .split('/')
            .next()
            .unwrap_or_default()
            .to_lowercase()
    }

    /// Check if this entry applies to a pre-prepared (lowercased, version-stripped) agent.
    fn applies_to_prepared(&self, ua_lower: &str) -> bool {
        for agent in &self.useragents {
            if agent == "*" || ua_lower.contains(agent.as_str()) {
                return true;
            }
        }
        false
    }

    /// check if this entry applies to the specified agent
    fn applies_to(&self, useragent: &str) -> bool {
        self.applies_to_prepared(&Self::prepare_useragent(useragent))
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

/// extract the path of a string
fn extract_path(url: &str) -> &str {
    if !url.is_empty() {
        let prefix = if url.starts_with("https://") {
            8
        } else if url.starts_with("http://") {
            7
        } else {
            0
        };

        let url_slice = &url[prefix..];

        if let Some(path_start) = url_slice.find('/') {
            let path = &url_slice[path_start..];

            if let Some(query_start) = path.find('?') {
                &path[..query_start]
            } else {
                path
            }
        } else {
            "/"
        }
    } else {
        "/"
    }
}

impl RobotFileParser {
    /// Establish a new robotparser for a website domain
    #[cfg(not(feature = "regex"))]
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

    /// Establish a new robotparser for a website domain
    #[cfg(feature = "regex")]
    pub fn new() -> Box<RobotFileParser> {
        RobotFileParser {
            entries: vec![],
            default_entry: Entry::new(),
            disallow_all: false,
            disallow_paths_regex: RegexSet::default(),
            disallow_agents_regex: RegexSet::default(),
            disallow_paths: Default::default(),
            disallow_agents: Default::default(),
            wild_card_agent: false,
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
        if let Ok(time) = SystemTime::now().duration_since(UNIX_EPOCH) {
            self.last_checked = time.as_secs() as i64;
        }
    }

    /// Get the entries inserted.
    pub fn get_entries(&self) -> &Vec<Entry> {
        &self.entries
    }

    /// Get the base entry inserted.
    pub fn get_base_entry(&self) -> &Entry {
        &self.default_entry
    }

    /// Reads the robots.txt URL and feeds it to the parser.
    pub async fn read(&mut self, client: &Client, url: &str) {
        use crate::client::StatusCode;
        self.modified();

        let request = client.get(string_concat!(url, "robots.txt"));

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
            _ => (),
        }
    }

    /// Reads the HTTP response and feeds it to the parser.
    pub async fn from_response(&mut self, response: crate::client::Response) {
        match response.text().await {
            Ok(buf) => {
                let lines: Vec<&str> = buf.split('\n').collect();

                self.parse(&lines);
            }
            _ => {
                self.allow_all = true;
            }
        }
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

        self.entries.reserve(lines.len() / 10);

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

            if let Some((left, right)) = ln.split_once(':') {
                let part0 = left.trim();
                let part1_raw = right.trim();
                let part1 =
                    String::from_utf8(percent_decode(part1_raw.as_bytes()).collect())
                        .unwrap_or_default();

                if part0.eq_ignore_ascii_case("user-agent") {
                    if state == 2 {
                        self._add_entry(entry);
                        entry = Entry::new();
                    }
                    entry.push_useragent(&part1);
                    state = 1;
                    self.set_disallow_agents_list(&part1);
                } else if part0.eq_ignore_ascii_case("disallow") {
                    if state != 0 {
                        entry.push_ruleline(RuleLine::new(&part1, false));
                        state = 2;
                        self.set_disallow_list(&part1);
                    }
                } else if part0.eq_ignore_ascii_case("allow") {
                    if state != 0 {
                        entry.push_ruleline(RuleLine::new(&part1, true));
                        state = 2;
                    }
                } else if part0.eq_ignore_ascii_case("crawl-delay") {
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
                } else if part0.eq_ignore_ascii_case("sitemap") {
                    if state != 0 {
                        state = 2;
                    }
                } else if part0.eq_ignore_ascii_case("request-rate") {
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
            }
        }

        if state == 2 {
            self._add_entry(entry);
        }

        self.build_disallow_list()
    }

    /// Include the disallow paths in the regex set. This does nothing without the 'regex' feature.
    #[cfg(not(feature = "regex"))]
    pub fn set_disallow_list(&mut self, _path: &str) {}

    /// Include the disallow  paths in the regex set. This does nothing without the 'regex' feature.
    #[cfg(feature = "regex")]
    pub fn set_disallow_list(&mut self, path: &str) {
        if !path.is_empty() {
            self.disallow_paths.insert(path.into());
        }
    }

    /// Include the disallow agents in the regex set. This does nothing without the 'regex' feature.
    #[cfg(not(feature = "regex"))]
    pub fn set_disallow_agents_list(&mut self, _agent: &str) {}

    /// Include the disallow agents in the regex set. This does nothing without the 'regex' feature.
    #[cfg(feature = "regex")]
    pub fn set_disallow_agents_list(&mut self, agent: &str) {
        if !agent.is_empty() {
            if agent == "*" {
                self.wild_card_agent = true;
            }
            self.disallow_agents.insert(agent.into());
        }
    }

    /// Build the regex disallow list. This does nothing without the 'regex' feature.
    #[cfg(not(feature = "regex"))]
    pub fn build_disallow_list(&mut self) {}

    /// Build the regex disallow list. This does nothing without the 'regex' feature.
    #[cfg(feature = "regex")]
    pub fn build_disallow_list(&mut self) {
        if !self.disallow_paths.is_empty() {
            match RegexSet::new(&self.disallow_paths) {
                Ok(s) => self.disallow_paths_regex = s,
                _ => (),
            }
        }
        if !self.disallow_agents.is_empty() {
            match RegexSet::new(&self.disallow_agents) {
                Ok(s) => self.disallow_agents_regex = s,
                _ => (),
            }
        }
    }

    /// Using the parsed robots.txt decide if useragent can fetch url
    pub fn can_fetch<T: AsRef<str>>(&self, useragent: T, url: &str) -> bool {
        // Until the robots.txt file has been read or found not
        // to exist, we must assume that no url is allowable.
        // This prevents false positives when a user erronenously
        // calls can_fetch() before calling read().
        if self.allow_all {
            true
        } else if self.last_checked == 0 || self.disallow_all {
            false
        } else {
            // search for given user agent matches
            // the first match counts
            let url_str = extract_path(url);

            if self.entry_allowed(&useragent, url_str) {
                true
            } else {
                // try the default entry last
                let default_entry = &self.default_entry;

                if !default_entry.is_empty() {
                    default_entry.allowance(url_str)
                } else {
                    // agent not found ==> access granted
                    true
                }
            }
        }
    }

    /// Is the entry apply to the robots.txt?
    #[cfg(not(feature = "regex"))]
    pub fn entry_allowed<T: AsRef<str>>(&self, useragent: &T, url_str: &str) -> bool {
        let ua_lower = Entry::prepare_useragent(useragent.as_ref());
        for entry in &self.entries {
            if entry.applies_to_prepared(&ua_lower) {
                return entry.allowance(url_str);
            }
        }
        false
    }

    /// Is the entry apply to the robots.txt?
    #[cfg(feature = "regex")]
    pub fn entry_allowed<T: AsRef<str>>(&self, useragent: &T, url_str: &str) -> bool {
        let agent_checked =
            self.wild_card_agent || self.disallow_agents_regex.is_match(useragent.as_ref());
        let disallow = agent_checked && self.disallow_paths_regex.is_match(url_str);

        !disallow
    }

    /// Returns the crawl delay for this user agent as a `Duration`, or None if no crawl delay is defined.
    pub fn get_crawl_delay(&self, useragent: &Option<Box<CompactString>>) -> Option<Duration> {
        if self.last_checked == 0 {
            None
        } else {
            let crawl_delay: Option<Duration> = match useragent.as_ref() {
                Some(ua) => {
                    let ua_lower = Entry::prepare_useragent(ua);
                    for entry in &self.entries {
                        if entry.applies_to_prepared(&ua_lower) {
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
        if self.last_checked == 0 {
            return None;
        }
        let ua_lower = Entry::prepare_useragent(useragent.as_ref());
        for entry in &self.entries {
            if entry.applies_to_prepared(&ua_lower) {
                return entry.get_req_rate();
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_path_basic() {
        assert_eq!(extract_path("https://example.com/foo/bar"), "/foo/bar");
    }

    #[test]
    fn test_extract_path_with_query() {
        assert_eq!(extract_path("https://example.com/foo?q=1"), "/foo");
    }

    #[test]
    fn test_extract_path_no_path() {
        assert_eq!(extract_path("https://example.com"), "/");
    }

    #[test]
    fn test_extract_path_empty() {
        assert_eq!(extract_path(""), "/");
    }

    #[test]
    fn test_extract_path_http() {
        assert_eq!(extract_path("http://example.com/page"), "/page");
    }

    #[test]
    fn test_extract_path_no_scheme() {
        assert_eq!(extract_path("example.com/page"), "/page");
    }

    #[cfg(not(feature = "regex"))]
    #[test]
    fn test_rule_line_applies_wildcard() {
        let rule = RuleLine::new("*", false);
        assert!(rule.applies_to("/anything"));
        assert!(rule.applies_to("/foo/bar"));
    }

    #[cfg(not(feature = "regex"))]
    #[test]
    fn test_rule_line_applies_prefix() {
        let rule = RuleLine::new("/foo*", false);
        assert!(rule.applies_to("/foobar"));
        assert!(rule.applies_to("/foo/baz"));
        assert!(!rule.applies_to("/bar"));
    }

    #[cfg(not(feature = "regex"))]
    #[test]
    fn test_rule_line_applies_exact() {
        let rule = RuleLine::new("/exact", false);
        assert!(rule.applies_to("/exact"));
        assert!(!rule.applies_to("/exact/more"));
        assert!(!rule.applies_to("/other"));
    }

    #[cfg(not(feature = "regex"))]
    #[test]
    fn test_rule_line_applies_directory() {
        let rule = RuleLine::new("/dir/", false);
        assert!(rule.applies_to("/dir/page"));
        assert!(rule.applies_to("/dir/sub/page"));
        assert!(!rule.applies_to("/other/"));
    }

    #[test]
    fn test_entry_applies_to_agent() {
        let mut entry = Entry::new();
        entry.push_useragent("googlebot");
        assert!(entry.applies_to("Googlebot"));
        assert!(entry.applies_to("Googlebot/2.1"));
        assert!(!entry.applies_to("Bingbot"));
    }

    #[test]
    fn test_entry_applies_to_wildcard_agent() {
        let mut entry = Entry::new();
        entry.push_useragent("*");
        assert!(entry.applies_to("Googlebot"));
        assert!(entry.applies_to("AnyAgent"));
    }

    #[cfg(not(feature = "regex"))]
    #[test]
    fn test_entry_allowance() {
        let mut entry = Entry::new();
        entry.push_useragent("*");
        entry.push_ruleline(RuleLine::new("/private", false));
        entry.push_ruleline(RuleLine::new("/public", true));

        assert!(!entry.allowance("/private"));
        assert!(entry.allowance("/public"));
        assert!(entry.allowance("/other"));
    }

    #[test]
    fn test_parser_basic() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        let lines = vec![
            "User-agent: *",
            "Disallow: /private",
            "Allow: /public",
        ];
        parser.parse(&lines);

        assert!(parser.can_fetch("Googlebot", "https://example.com/public"));
    }

    #[test]
    fn test_parser_multiple_agents() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        let lines = vec![
            "User-agent: googlebot",
            "Disallow: /nogoogle",
            "",
            "User-agent: bingbot",
            "Disallow: /nobing",
        ];
        parser.parse(&lines);

        let entries = parser.get_entries();
        assert!(entries.len() >= 1);
    }

    #[test]
    fn test_parser_crawl_delay() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        let lines = vec![
            "User-agent: testbot",
            "Crawl-delay: 5",
            "Disallow: /test",
        ];
        parser.parse(&lines);

        let entries = parser.get_entries();
        assert!(!entries.is_empty());
        let entry = &entries[0];
        assert_eq!(entry.crawl_delay, Some(Duration::from_secs(5)));
    }

    #[test]
    fn test_parser_request_rate() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        let lines = vec![
            "User-agent: testbot",
            "Request-rate: 3/60",
            "Disallow: /test",
        ];
        parser.parse(&lines);

        let rate = parser.get_req_rate("testbot");
        assert!(rate.is_some());
        let rate = rate.unwrap();
        assert_eq!(rate.requests, 3);
        assert_eq!(rate.seconds, 60);
    }

    #[test]
    fn test_parser_disallow_all() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        parser.disallow_all = true;
        assert!(!parser.can_fetch("*", "https://example.com/any"));
    }

    #[test]
    fn test_parser_allow_all() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        parser.allow_all = true;
        assert!(parser.can_fetch("*", "https://example.com/any"));
    }

    #[test]
    fn test_parser_comments() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        let lines = vec![
            "# This is a comment",
            "User-agent: * # all bots",
            "Disallow: /secret # hidden area",
        ];
        parser.parse(&lines);

        let base = parser.get_base_entry();
        assert!(base.has_useragent());
    }

    #[cfg(not(feature = "regex"))]
    #[test]
    fn test_parser_empty_disallow() {
        let rule = RuleLine::new("", false);
        assert!(rule.allowance);
    }

    #[cfg(not(feature = "regex"))]
    #[test]
    fn test_can_fetch_case_insensitive() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        let lines = vec![
            "User-agent: googlebot",
            "Disallow: /private",
        ];
        parser.parse(&lines);

        // entry_allowed correctly tests case-insensitive matching
        assert!(!parser.entry_allowed(&"GoogleBot", "/private"));
        assert!(!parser.entry_allowed(&"googlebot", "/private"));
        assert!(!parser.entry_allowed(&"GOOGLEBOT", "/private"));
        // Allowed path works for all cases too
        assert!(parser.entry_allowed(&"GoogleBot", "/public"));
    }

    #[cfg(not(feature = "regex"))]
    #[test]
    fn test_can_fetch_with_version() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        let lines = vec![
            "User-agent: googlebot",
            "Disallow: /secret",
        ];
        parser.parse(&lines);

        // "Googlebot/2.1" should match "googlebot" entry (version stripped)
        assert!(!parser.entry_allowed(&"Googlebot/2.1", "/secret"));
        assert!(parser.entry_allowed(&"Googlebot/2.1", "/public"));
    }

    #[cfg(not(feature = "regex"))]
    #[test]
    fn test_can_fetch_multiple_entries() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        let lines = vec![
            "User-agent: googlebot",
            "Disallow: /nogoogle",
            "",
            "User-agent: bingbot",
            "Disallow: /nobing",
            "",
            "User-agent: duckduckbot",
            "Disallow: /noduck",
        ];
        parser.parse(&lines);

        let entries = parser.get_entries();
        // All 3 specific entries should be present
        assert_eq!(entries.len(), 3);

        // Verify correct entry is matched for each agent via entry_allowed
        // (entry_allowed returns the allowance result directly)
        assert!(!parser.entry_allowed(&"Googlebot", "/nogoogle"));
        assert!(parser.entry_allowed(&"Googlebot", "/public"));
        assert!(!parser.entry_allowed(&"Bingbot", "/nobing"));
        assert!(parser.entry_allowed(&"Bingbot", "/public"));
        assert!(!parser.entry_allowed(&"DuckDuckBot", "/noduck"));
        assert!(parser.entry_allowed(&"DuckDuckBot", "/public"));
        // Cross-agent: googlebot should not match bingbot's rules
        assert!(parser.entry_allowed(&"Googlebot", "/nobing"));
    }

    #[test]
    fn test_get_crawl_delay_case_insensitive() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        let lines = vec![
            "User-agent: slowbot",
            "Crawl-delay: 10",
            "Disallow: /test",
        ];
        parser.parse(&lines);

        let ua = Some(Box::new(CompactString::new("SlowBot/1.0")));
        let delay = parser.get_crawl_delay(&ua);
        assert_eq!(delay, Some(Duration::from_secs(10)));

        let ua_upper = Some(Box::new(CompactString::new("SLOWBOT")));
        let delay_upper = parser.get_crawl_delay(&ua_upper);
        assert_eq!(delay_upper, Some(Duration::from_secs(10)));
    }

    #[test]
    fn test_get_req_rate_agent_match() {
        let mut parser = RobotFileParser::new();
        parser.modified();
        let lines = vec![
            "User-agent: fastbot",
            "Request-rate: 5/30",
            "Disallow: /test",
            "",
            "User-agent: slowbot",
            "Request-rate: 1/60",
            "Disallow: /test",
        ];
        parser.parse(&lines);

        let fast_rate = parser.get_req_rate("FastBot/2.0");
        assert!(fast_rate.is_some());
        let fr = fast_rate.unwrap();
        assert_eq!(fr.requests, 5);
        assert_eq!(fr.seconds, 30);

        let slow_rate = parser.get_req_rate("SLOWBOT");
        assert!(slow_rate.is_some());
        let sr = slow_rate.unwrap();
        assert_eq!(sr.requests, 1);
        assert_eq!(sr.seconds, 60);

        // Unknown agent should return None
        assert!(parser.get_req_rate("unknownbot").is_none());
    }
}
