use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn redirect_1_301() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/redirect_1").await;
    assert_eq!(
        page.status_code,
        StatusCode::OK,
        "301 should resolve to 200"
    );
    assert!(
        page.final_redirect_destination.is_some(),
        "should track redirect destination"
    );
}

#[tokio::test]
async fn redirect_2() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/redirect_2").await;
    assert!(
        page.status_code.is_success(),
        "redirect_2 should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn redirect_3_302() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/redirect_3_302").await;
    assert_eq!(
        page.status_code,
        StatusCode::OK,
        "302 should resolve to 200"
    );
}

#[tokio::test]
async fn redirect_4_307() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/redirect_4_307").await;
    assert_eq!(
        page.status_code,
        StatusCode::OK,
        "307 should resolve to 200"
    );
}

#[tokio::test]
async fn redirect_300() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http_no_redirect("/redirects/redirect_300").await;
    let s = page.status_code.as_u16();
    assert!(s == 300 || s == 200, "redirect_300: got {}", s);
}

#[tokio::test]
async fn redirect_303() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/redirect_303").await;
    assert!(
        page.status_code.is_success() || page.status_code.is_redirection(),
        "303: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn redirect_304() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http_no_redirect("/redirects/redirect_304").await;
    let s = page.status_code.as_u16();
    assert!(s == 304 || s == 200, "redirect_304: got {}", s);
}

#[tokio::test]
async fn redirect_305() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http_no_redirect("/redirects/redirect_305").await;
    let s = page.status_code.as_u16();
    assert!(s == 305 || s == 200, "redirect_305: got {}", s);
}

#[tokio::test]
async fn redirect_306() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http_no_redirect("/redirects/redirect_306").await;
    let s = page.status_code.as_u16();
    assert!(s == 306 || s == 200, "redirect_306: got {}", s);
}

#[tokio::test]
async fn redirect_308() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/redirect_308").await;
    assert_eq!(
        page.status_code,
        StatusCode::OK,
        "308 should resolve to 200"
    );
}

#[tokio::test]
async fn disallowed_redirect() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/disallowed_redirect").await;
    // Should follow to destination regardless of robots
    assert!(
        page.status_code.is_success() || page.status_code.is_redirection(),
        "disallowed_redirect: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn disallowed_redirect_target() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/disallowed_redirect_target_redirect").await;
    assert!(
        page.status_code.is_success() || page.status_code.is_redirection(),
        "disallowed_redirect_target: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn redirect_chain_allowed() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/redirect_chain_allowed").await;
    assert!(
        page.status_code.is_success() || page.status_code.is_redirection(),
        "redirect chain: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn redirect_to_404() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/redirect_to_404").await;
    assert_eq!(page.status_code, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn redirect_content() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/redirect_content").await;
    assert!(page.status_code.is_success());
    assert!(
        !page.get_html().is_empty(),
        "redirect target should have content"
    );
}

#[tokio::test]
async fn external_redirect() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/external_redirect").await;
    // External redirect may resolve or be blocked by redirect policy
    let _status = page.status_code;
}

#[tokio::test]
async fn external_redirect_chain1() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/external_redirect_chain1").await;
    let _status = page.status_code;
}

#[tokio::test]
async fn invalid_redirect() {
    if !run_live_tests() {
        return;
    }
    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(15),
        fetch_page_http("/redirects/invalid_redirect"),
    )
    .await;
    assert!(result.is_ok(), "invalid redirect should not hang");
}

#[tokio::test]
async fn url_redirect_chains() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/url_redirect_chains").await;
    assert!(
        page.status_code.is_success() || page.status_code.is_redirection(),
        "url_redirect_chains: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn infinite_redirect_does_not_hang() {
    if !run_live_tests() {
        return;
    }
    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(15),
        fetch_page_http("/redirects/infinite_redirect"),
    )
    .await;
    assert!(result.is_ok(), "infinite redirect should not hang");
    let page = result.unwrap();
    assert!(
        !page.status_code.is_success(),
        "infinite redirect should not resolve to success, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn two_step_redirect_loop() {
    if !run_live_tests() {
        return;
    }
    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(15),
        fetch_page_http("/redirects/two_step_redirect_loop_1"),
    )
    .await;
    assert!(result.is_ok(), "redirect loop should not hang");
}

#[tokio::test]
async fn redirect_301_no_follow() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http_no_redirect("/redirects/redirect_1").await;
    assert_eq!(page.status_code, StatusCode::MOVED_PERMANENTLY);
}

// --- Meta redirects (HTML-based, HTTP sees them as 200 with meta refresh tag) ---

#[tokio::test]
async fn meta_redirect_1_http() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/meta_redirect_1").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("refresh") || html.contains("redirect"),
        "meta_redirect_1 should have refresh/redirect"
    );
}

#[tokio::test]
async fn meta_redirect_2_http() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/meta_redirect_2").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn meta_redirect_3_http() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/meta_redirect_3").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn infinite_meta_redirect() {
    if !run_live_tests() {
        return;
    }
    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(15),
        fetch_page_http("/redirects/infinite_meta_redirect"),
    )
    .await;
    assert!(result.is_ok(), "infinite meta redirect should not hang");
}

#[tokio::test]
async fn external_meta_redirect() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/external_meta_redirect").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn invalid_meta_redirect() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/invalid_meta_redirect").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn header_refresh_redirect() {
    if !run_live_tests() {
        return;
    }
    let page = fetch_page_http("/redirects/header_refresh_redirect").await;
    // May follow or return the page with the Refresh header
    assert!(
        page.status_code.is_success() || page.status_code.is_redirection(),
        "header_refresh_redirect: got {}",
        page.status_code
    );
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn meta_redirect_chrome() {
    if !run_live_tests() {
        return;
    }
    if let Some(page) = fetch_page_chrome("/redirects/meta_redirect_1").await {
        assert!(
            !page.get_html().is_empty(),
            "meta redirect should produce content in chrome"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}
