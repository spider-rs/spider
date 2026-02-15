use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn broken_links_internal_page_exists() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/broken_links_internal").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    assert!(
        html.contains("<a ") || html.contains("<A "),
        "broken_links_internal page should contain anchor tags"
    );
}

#[tokio::test]
async fn broken_links_external_page_exists() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/broken_links_external").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    assert!(
        html.contains("href="),
        "broken_links_external page should contain links"
    );
}

#[tokio::test]
async fn max_external_links() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/max_external_links").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("href="),
        "max_external_links should have links"
    );
}

#[tokio::test]
async fn page_with_external_links() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/page_with_external_links").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("http://") || html.contains("https://"),
        "should contain external links"
    );
}

#[tokio::test]
async fn nofollowed_page() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/nofollowed_page").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn nofollow_link_with_nofollowed_backlinks() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/nofollow_link_with_nofollowed_backlinks").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn relative_link_resolution() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/relative_link/a/b").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn relative_link_with_base_tag() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/relative_link_with_base/a/b").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("<base "),
        "page should have a <base> tag for relative URL resolution"
    );
}

#[tokio::test]
async fn image_links() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/image_links").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("<img "),
        "image_links page should contain img tags"
    );
}

#[tokio::test]
async fn non_default_language() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/non_default_language").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn meta_refresh() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/meta_refresh").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn header_refresh() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/header_refresh").await;
    assert!(
        page.status_code.is_success() || page.status_code.is_redirection(),
        "header_refresh: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn external_links_to_disallowed_urls() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/external_links_to_disallwed_urls").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn non_standard_links() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/non_standard_links").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn repeated_external_links() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/repeated_external_links").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn repeated_internal_links() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/repeated_internal_links").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn links_with_quote_variations() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/links_with_quote_variations").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    assert!(html.contains("href="), "should have href attributes");
}

#[tokio::test]
async fn whitespace_in_links() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/whitespace_in_links").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn comma_separated_attributes() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/comma_separated_attributes").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn nofollow_and_follow_link() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/links/nofollow_and_follow_link").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn relative_protocol_page() {
    if !run_live_tests() {
        return;
    }

    // Protocol-relative URL — use absolute URL directly
    let page = fetch_page_http("/links/relative_protocol_page").await;
    assert!(
        page.status_code.is_success(),
        "relative_protocol_page: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn crawl_discovers_links() {
    if !run_live_tests() {
        return;
    }

    let pages = crawl_collect_http("/links/page_with_external_links", 3, 1).await;
    assert!(
        !pages.is_empty(),
        "crawl should discover at least the seed page"
    );
}

#[cfg(feature = "smart")]
#[tokio::test]
async fn links_smart_mode() {
    if !run_live_tests() {
        return;
    }

    let pages = crawl_collect_smart("/links/page_with_external_links", 3, 1).await;
    assert!(
        !pages.is_empty(),
        "smart crawl should discover at least the seed page"
    );
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn links_chrome_mode() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/links/page_with_external_links").await {
        assert!(
            !page.get_html().is_empty(),
            "chrome should render the links page"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}
