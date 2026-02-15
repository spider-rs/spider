use crate::helpers::*;
use spider::hashbrown::HashMap;
use spider::reqwest::StatusCode;
use spider::website::Website;

// --- Double slash variants ---

#[tokio::test]
async fn double_slash_urls() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/urls/double_slash//one",
        "/urls/double_slash////two",
        "/urls/double_slash//////three",
        "/urls/double_slash//////four//",
    ];

    for path in &paths {
        let page = fetch_page_http(path).await;
        assert!(
            page.status_code.is_success() || page.status_code == StatusCode::NOT_FOUND,
            "double slash URL {} should resolve, got {}",
            path,
            page.status_code
        );
    }
}

#[tokio::test]
async fn double_slash_disallowed_variants() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/urls/double_slash//disallowed_middle",
        "/urls/double_slash/disallowed_end//",
    ];

    for path in &paths {
        let page = fetch_page_http(path).await;
        assert!(
            page.status_code.is_success() || page.status_code == StatusCode::NOT_FOUND,
            "disallowed double slash {} : got {}",
            path,
            page.status_code
        );
    }
}

// --- Parameter variants ---

#[tokio::test]
async fn url_with_parameters() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/urls/parameter_1_1?parameter_1=foo",
        "/urls/parameter_1_2?parameter_x=x&parameter_1=foo",
        "/urls/parameter_1_3?parameter_x=x&parameter_1=foo&parameter_y=y",
        "/urls/parameter_2_1?parameter_1=foo",
        "/urls/parameter_2_2?parameter_x=x&parameter_1=foo",
        "/urls/parameter_2_3?parameter_x=x&parameter_1=foo&parameter_y=y",
    ];

    for path in &paths {
        let page = fetch_page_http(path).await;
        assert!(
            page.status_code.is_success(),
            "URL with parameters {} should resolve, got {}",
            path,
            page.status_code
        );
    }
}

#[tokio::test]
async fn duplicate_parameter_values() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/parameter?parameter_1=foo&parameter_1=bar").await;
    assert!(
        page.status_code.is_success(),
        "duplicate param different values: got {}",
        page.status_code
    );

    let page2 = fetch_page_http("/urls/parameter?parameter_1=foo&parameter_1=foo").await;
    assert!(
        page2.status_code.is_success(),
        "duplicate param same values: got {}",
        page2.status_code
    );
}

// --- Spaces and encoding ---

#[tokio::test]
async fn url_with_spaces() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/url_with_spaces/URL%20with%20spaces").await;
    assert!(
        page.status_code.is_success(),
        "URL with encoded spaces should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_trailing_space() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/url_with_trailing_space/%20").await;
    assert!(
        page.status_code.is_success(),
        "trailing space URL: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_encoded_trailing_space() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/url_with_encoded_trailing_space/").await;
    assert!(
        page.status_code.is_success(),
        "encoded trailing space URL: got {}",
        page.status_code
    );
}

// --- Duplication types ---

#[tokio::test]
async fn url_with_trailing_slash_vs_without() {
    if !run_live_tests() {
        return;
    }

    let with_slash = fetch_page_http("/urls/duplication_types/").await;
    let without_slash = fetch_page_http("/urls/duplication_types").await;

    assert!(with_slash.status_code.is_success(), "with trailing slash");
    assert!(
        without_slash.status_code.is_success(),
        "without trailing slash"
    );
}

#[tokio::test]
async fn duplication_types_tracking() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/duplication_types?tracking=yes").await;
    assert!(
        page.status_code.is_success(),
        "duplication with tracking param: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn duplication_types_index_html() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/duplication_types/index.html").await;
    assert!(
        page.status_code.is_success(),
        "duplication_types/index.html: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn duplication_types_nested() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/duplication_types/duplication_types/").await;
    assert!(
        page.status_code.is_success(),
        "nested duplication_types: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn duplication_types_uppercase() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/DUPLICATION_TYPES").await;
    assert!(
        page.status_code.is_success(),
        "uppercase DUPLICATION_TYPES: got {}",
        page.status_code
    );
}

// --- Fragment and encoding ---

#[tokio::test]
async fn url_with_fragment() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/url_with_fragment").await;
    assert!(
        page.status_code.is_success(),
        "URL with fragment should resolve"
    );
}

#[tokio::test]
async fn url_with_encoded_characters() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/url/URL%2c_with_encoded_reserved_character").await;
    assert!(
        page.status_code.is_success(),
        "URL with encoded characters should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_encoded_unreserved_character() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/url/URL_with_encoded_unreserved_%63haracter").await;
    assert!(
        page.status_code.is_success(),
        "URL with encoded unreserved char: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_encoded_space() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/url/with_encoded_space").await;
    assert!(
        page.status_code.is_success(),
        "URL with encoded space: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_encoded_o_character() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/url/with_encoded_%C3%B3_character").await;
    assert!(
        page.status_code.is_success(),
        "URL with encoded ó: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_colon() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/url/url_with:colon").await;
    assert!(
        page.status_code.is_success(),
        "URL with colon in path should resolve, got {}",
        page.status_code
    );
}

// --- Session ID ---

#[tokio::test]
async fn url_with_session_id() {
    if !run_live_tests() {
        return;
    }

    let page =
        fetch_page_http("/urls/with_session_id?sessionID=5vSrxOoHw90aGk81xRa6").await;
    assert!(
        page.status_code.is_success(),
        "URL with session ID should resolve"
    );
}

// --- Directory index ---

#[tokio::test]
async fn directory_index_variants() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/urls/directory_index/",
        "/urls/directory_index/index.html",
        "/urls/directory_index/index.htm",
        "/urls/directory_index/default.htm",
        "/urls/directory_index/index",
    ];

    for path in &paths {
        let page = fetch_page_http(path).await;
        assert!(
            page.status_code.is_success(),
            "directory index {} should resolve, got {}",
            path,
            page.status_code
        );
    }
}

// --- Malformed URLs ---

#[tokio::test]
async fn links_to_malformed_urls() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/links_to_malformed_urls").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- Pagination ---

#[tokio::test]
async fn paginated_pages() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/paginated_page").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("rel=\"next\"") || html.contains("rel=\"prev\"") || html.contains("page"),
        "paginated page should have pagination markers"
    );
}

#[tokio::test]
async fn unlinked_paginated_page() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/unlinked_paginated_page").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn paginated_and_noindex_page() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/paginated_and_noindex_page").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- Non-HTML file types ---

#[tokio::test]
async fn links_to_non_html_filetypes() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/links_to_non_html_filetypes").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains(".pdf") || html.contains(".jpg") || html.contains(".png"),
        "should contain links to non-HTML file types"
    );
}

// --- Hreflang ---

#[tokio::test]
async fn pages_with_hreflang() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/pages_with_hreflang").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("hreflang"),
        "should contain hreflang attributes"
    );
}

#[tokio::test]
async fn page_with_hreflang_header_ok() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/page_with_hreflang_header_ok").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn page_with_hreflang_header_not_ok() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/page_with_hreflang_header_not_ok").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- Duplicate pages ---

#[tokio::test]
async fn duplicate_pages() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/urls/duplicate_page",
        "/urls/duplicate_page/foo",
        "/urls/duplicate_page/bar",
        "/urls/duplicate_page/baz",
    ];

    for path in &paths {
        let page = fetch_page_http(path).await;
        assert_eq!(
            page.status_code,
            StatusCode::OK,
            "duplicate_page {} should resolve",
            path
        );
    }
}

// --- Page URL length ---

#[tokio::test]
async fn page_url_length_n() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/urls/page_url_length_n").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- Multiple slashes ---

#[tokio::test]
async fn multiple_slashes() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/urls/multiple_slashes///200_404",
        "/urls/multiple_slashes///404_200",
    ];

    for path in &paths {
        let page = fetch_page_http(path).await;
        let status = page.status_code.as_u16();
        assert!(
            status == 200 || status == 404,
            "multiple slashes URL {}: got {}",
            path,
            status
        );
    }
}

// --- Deep paths ---

#[tokio::test]
async fn deep_paths() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/one/two/three/four",
        "/one/two/three/four/five",
        "/one/two/three/four/five/six",
        "/one/two/three/four/five/six/seven",
    ];

    for path in &paths {
        let page = fetch_page_http(path).await;
        assert!(
            page.status_code.is_success(),
            "deep path {} : got {}",
            path,
            page.status_code
        );
    }
}

#[tokio::test]
async fn path_segments() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/path/1/path/2",
        "/path/1/path/2/path/3",
    ];

    for path in &paths {
        let page = fetch_page_http(path).await;
        assert!(
            page.status_code.is_success(),
            "path {} : got {}",
            path,
            page.status_code
        );
    }
}

// --- Relative base source ---

#[tokio::test]
async fn relative_base_source() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/relative_base_source").await;
    assert!(
        page.status_code.is_success(),
        "relative_base_source: got {}",
        page.status_code
    );
}

// --- Parameter on hostname root ---

#[tokio::test]
async fn parameter_on_hostname_root() {
    if !run_live_tests() {
        return;
    }

    let page =
        fetch_page_http("/?parameter-on-hostname-root=parameter-value").await;
    assert!(
        page.status_code.is_success(),
        "parameter on root: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn removed_and_retained_parameter() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/?removed_parameter=1&retained_parameter=1").await;
    assert!(
        page.status_code.is_success(),
        "removed/retained params: got {}",
        page.status_code
    );
}

// --- Infinite URL budget ---

#[tokio::test]
async fn infinite_urls_budget_caps_crawl() {
    if !run_live_tests() {
        return;
    }

    let budget = 5u32;
    let url = format!("{}/infinite/", BASE);
    let mut website = Website::new(&url);
    website
        .with_budget(Some(HashMap::from([("*", budget)])))
        .with_depth(3)
        .with_request_timeout(Some(std::time::Duration::from_secs(30)))
        .with_crawl_timeout(Some(std::time::Duration::from_secs(60)));

    let mut w = website.clone();
    let mut rx = w.subscribe(16).expect("subscribe");
    let (done_tx, mut done_rx) = spider::tokio::sync::oneshot::channel::<()>();

    let crawl = async move {
        w.crawl_raw().await;
        w.unsubscribe();
        let _ = done_tx.send(());
    };

    let mut urls = Vec::new();
    let sub = async {
        loop {
            spider::tokio::select! {
                biased;
                _ = &mut done_rx => break,
                result = rx.recv() => {
                    if let Ok(page) = result {
                        urls.push(page.get_url().to_string());
                    } else {
                        break;
                    }
                }
            }
        }
    };

    let crawl_result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(90),
        async { spider::tokio::join!(sub, crawl) },
    )
    .await;

    assert!(crawl_result.is_ok(), "infinite crawl should not hang");

    assert!(
        urls.len() <= (budget as usize) + 2,
        "budget should cap infinite crawl; got {} pages",
        urls.len()
    );
}

// --- Depth limit ---

#[tokio::test]
async fn depth_limit_respected() {
    if !run_live_tests() {
        return;
    }

    let pages = crawl_collect_http("/one/two/three/four", 10, 2).await;
    assert!(
        pages.len() <= 12,
        "depth limit should constrain crawl; got {} pages",
        pages.len()
    );
}

// --- Chrome ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn url_with_spaces_chrome() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/urls/url_with_spaces/URL%20with%20spaces").await {
        assert!(
            !page.get_html().is_empty(),
            "chrome should handle URL with spaces"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}
