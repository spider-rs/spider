use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn responsive() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/responsive").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("viewport"),
        "responsive page should have a viewport meta tag"
    );
}

#[tokio::test]
async fn dynamic_mobile() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/dynamic").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn separate_desktop() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_desktop").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn no_mobile_configuration() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/no_mobile_configuration").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn responsive_with_amp() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/responsive_with_amp").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("amphtml") || html.contains("viewport"),
        "responsive_with_amp should have amp or viewport markers"
    );
}

// --- Separate desktop variants ---

#[tokio::test]
async fn separate_desktop_with_different_h1() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_desktop_with_different_h1").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn separate_desktop_with_different_title() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_desktop_with_different_title").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn separate_desktop_with_different_wordcount() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_desktop_with_different_wordcount").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn separate_desktop_with_different_links_in() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_desktop_with_different_links_in").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn separate_desktop_with_different_links_out() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_desktop_with_different_links_out").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn separate_desktop_with_mobile_not_subdomain() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_desktop_with_mobile_not_subdomain").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn separate_mobile_with_mobile_not_subdomain() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_mobile_with_mobile_not_subdomain").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn separate_desktop_irregular_media() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_desktop_irregular_media").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn separate_desktop_response_header_alt() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_desktop_response_header_alt").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- AMP variants ---

#[tokio::test]
async fn desktop_with_amp_as_mobile() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/desktop_with_AMP_as_mobile").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn desktop_with_self_canonical_mobile_and_amp() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/desktop_with_self_canonical_mobile_and_amp").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn other_desktop_that_links_to_same_mobile_pages() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/other_desktop_that_links_to_the_same_mobile_pages").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn amp_with_separate_mobile() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/amp_with_separate_mobile").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn amp_with_responsive() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/amp_with_responsive").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn no_mobile_with_amp() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/no_mobile_with_amp").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn amp_with_no_mobile() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/amp_with_no_mobile").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn amp_no_references() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/amp_no_references").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn amp_as_desktop_amp_and_mobile() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/amp_as_desktop_amp_and_mobile").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn separate_amp_with_self_canonical() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/mobile/separate_amp_with_self_canonical").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- Chrome ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn responsive_chrome() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/mobile/responsive").await {
        let html = page.get_html().to_lowercase();
        assert!(
            html.contains("viewport"),
            "chrome should see viewport meta tag"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn dynamic_mobile_chrome() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/mobile/dynamic").await {
        assert!(
            !page.get_html().is_empty(),
            "chrome should render dynamic mobile page"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}
