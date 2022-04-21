pub use crate::reqwest::blocking::{Client};
use reqwest::StatusCode;

pub fn fetch_page_html(url: &str, client: &Client) -> String {
    let mut body = String::new();

    // silence errors for top level logging
    match client.get(url).send() {
        Ok(res) if res.status() == StatusCode::OK => match res.text() {
            Ok(text) => body = text,
            Err(_) => {},
        },
        Ok(_) => (),
        Err(_) => {}
    }

    body
}
