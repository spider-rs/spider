use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn open_graph_tags() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/social_tags/open_graph_tags").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();

    assert!(
        html.contains("og:title"),
        "open_graph_tags page should have og:title"
    );
    assert!(
        html.contains("og:description"),
        "open_graph_tags page should have og:description"
    );
    assert!(
        html.contains("og:image"),
        "open_graph_tags page should have og:image"
    );
}

#[tokio::test]
async fn twitter_card_page_1() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/social_tags/twitter_card_page/1").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("twitter:card") || html.contains("twitter:title"),
        "twitter_card_page/1 should have Twitter Card meta tags"
    );
}

#[tokio::test]
async fn twitter_card_page_2() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/social_tags/twitter_card_page/2").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("twitter:card") || html.contains("twitter:title"),
        "twitter_card_page/2 should have Twitter Card meta tags"
    );
}

#[tokio::test]
async fn og_no_twitter() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/social_tags/og_no_twitter").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("og:title"),
        "og_no_twitter should have OG tags"
    );
    let has_twitter = html.contains("twitter:card");
    assert!(!has_twitter, "og_no_twitter should not have twitter:card");
}

#[tokio::test]
async fn max_twitter_card_description_length() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/social_tags/max_twitter_card_description_length").await;
    assert_eq!(page.status_code, StatusCode::OK);
}
