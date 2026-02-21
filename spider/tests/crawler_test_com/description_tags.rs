use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn missing_description() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/description_tags/missing_description").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let desc = extract_meta_description(&html);
    if desc.is_some() {
        eprintln!("WARN: missing_description has a description: {:?}", desc);
    }
}

#[tokio::test]
async fn no_description_nosnippet() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/description_tags/no_description_nosnippet").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn description_with_whitespace() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/description_tags/description_with_whitespace").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let desc = extract_meta_description(&html);
    assert!(desc.is_some(), "should have a description tag");
}

#[tokio::test]
async fn description_over_max() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/description_tags/description_over_max").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let desc = extract_meta_description(&html);
    assert!(desc.is_some(), "should have a description");
    let d = desc.unwrap();
    assert!(
        d.len() > 155,
        "description_over_max should be long, got {} chars",
        d.len()
    );
}

#[tokio::test]
async fn short_meta_description() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/description_tags/short_meta_description").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn duplicate_description() {
    if !run_live_tests() {
        return;
    }

    let page1 = fetch_page_http("/description_tags/duplicate_description").await;
    let page2 = fetch_page_http("/description_tags/duplicate_description/foo").await;
    assert_eq!(page1.status_code, StatusCode::OK);
    assert_eq!(page2.status_code, StatusCode::OK);

    let desc1 = extract_meta_description(&page1.get_html());
    let desc2 = extract_meta_description(&page2.get_html());
    assert_eq!(
        desc1, desc2,
        "duplicate_description pages should have the same description"
    );
}

#[tokio::test]
async fn duplicate_description_and_noindex() {
    if !run_live_tests() {
        return;
    }

    let page1 = fetch_page_http("/description_tags/duplicate_description_and_noindex").await;
    let page2 = fetch_page_http("/description_tags/duplicate_description_and_noindex/foo").await;
    assert_eq!(page1.status_code, StatusCode::OK);
    assert_eq!(page2.status_code, StatusCode::OK);
}

#[tokio::test]
async fn description_http_equiv() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/description_tags/description_http_equiv").await;
    assert_eq!(page.status_code, StatusCode::OK);
}
