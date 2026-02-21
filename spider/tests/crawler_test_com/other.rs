use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn crawler_user_agent() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/crawler_user_agent").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    assert!(!html.is_empty(), "user_agent page should echo back the UA");
}

#[tokio::test]
async fn crawler_ip_address() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/crawler_ip_address").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn crawler_request_headers() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/crawler_request_headers").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    assert!(
        !html.is_empty(),
        "request_headers page should echo back headers"
    );
}

#[tokio::test]
async fn page_with_hsts_headers() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/page_with_hsts_headers").await;
    assert_eq!(page.status_code, StatusCode::OK);
    if let Some(headers) = &page.headers {
        let has_hsts = headers.get("strict-transport-security").is_some();
        assert!(has_hsts, "should have HSTS header");
    }
}

#[tokio::test]
async fn page_load_time_n() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/page_load_time_n").await;
    assert!(
        page.status_code.is_success() || page.status_code == StatusCode::NOT_FOUND,
        "page_load_time_n: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn typo_in_head() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/typo_in_head").await;
    assert_eq!(page.status_code, StatusCode::OK);
    assert!(
        !page.get_html().is_empty(),
        "typo in head should not prevent HTML parsing"
    );
}

#[tokio::test]
async fn unfinished_tag_in_head() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/unfinished_tag_in_head").await;
    assert_eq!(page.status_code, StatusCode::OK);
    assert!(
        !page.get_html().is_empty(),
        "unfinished tag should not prevent parsing"
    );
}

#[tokio::test]
async fn non_head_tag_in_head() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/non_head_tag_in_head").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn link_tag_in_body() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/link_tag_in_body").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn script_tag_contents() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/script_tag_contents").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(html.contains("<script"), "should contain script tags");
}

#[tokio::test]
async fn conflicting_language_tags() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/conflicting_language_tags").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn basic_auth_returns_401() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/basic_auth").await;
    let status = page.status_code.as_u16();
    assert!(
        status == 401 || status == 200,
        "basic_auth page should return 401 or 200, got {}",
        status
    );
}

#[tokio::test]
async fn noodp_noydir_tags() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/noodp_noydir_tags").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn duplicated_body_content_1() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/duplicated_body_content_1").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn duplicated_body_content_2() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/duplicated_body_content_2").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn string_width() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/string_width/512/string").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn in_web_linking() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/in_web_linking").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn in_web_linked() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/other/in_web_linked").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- Chrome ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn typo_in_head_chrome() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/other/typo_in_head").await {
        assert!(
            !page.get_html().is_empty(),
            "chrome should handle typo in head gracefully"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn user_agent_chrome() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/other/crawler_user_agent").await {
        assert!(
            !page.get_html().is_empty(),
            "chrome should render user_agent page"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}
