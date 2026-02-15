use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn robots_excluded_page_accessible() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/robots_excluded").await;
    assert_eq!(
        page.status_code,
        StatusCode::OK,
        "robots excluded page still returns 200 via HTTP"
    );
}

#[tokio::test]
async fn deepcrawl_excluded() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/deepcrawl_excluded").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn robots_excluded_duplicate_description() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/robots_excluded_duplicate_description").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn robots_excluded_meta_noindex() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/robots_excluded_meta_noindex").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn deepcrawl_ua_disallow() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/deepcrawl_ua_disallow/foo").await;
    assert!(
        page.status_code.is_success() || page.status_code == StatusCode::NOT_FOUND,
        "deepcrawl_ua_disallow: got {}",
        page.status_code
    );
}

#[tokio::test]
async fn user_excluded() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/user_excluded").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn meta_noindex() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/meta_noindex").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("noindex"),
        "meta_noindex page should contain noindex directive"
    );
}

#[tokio::test]
async fn meta_noindex_uppercase() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/meta_noindex_uppercase").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("noindex"),
        "uppercase noindex should still be detectable"
    );
}

#[tokio::test]
async fn meta_nofollow() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/meta_nofollow").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("nofollow"),
        "meta_nofollow page should contain nofollow directive"
    );
}

#[tokio::test]
async fn meta_noarchive() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/meta_noarchive").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn x_robots_tag_noindex() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/x_robots_tag_noindex").await;
    assert_eq!(page.status_code, StatusCode::OK);
    if let Some(headers) = &page.headers {
        let x_robots = headers
            .get("x-robots-tag")
            .map(|v| v.to_str().unwrap_or("").to_lowercase());
        assert!(
            x_robots.map_or(false, |v| v.contains("noindex")),
            "should have X-Robots-Tag: noindex header"
        );
    }
}

#[tokio::test]
async fn page_allowed_with_robots() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/page_allowed_with_robots").await;
    assert_eq!(
        page.status_code,
        StatusCode::OK,
        "allowed page should be accessible"
    );
}

#[tokio::test]
async fn robots_noindexed() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/robots_noindexed").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn robots_noindex_conflict() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/robots_noindex_conflict").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn robots_excluded_blank_line() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/robots_excluded_blank_line").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn robots_noindexed_and_robots_disallowed() {
    if !run_live_tests() {
        return;
    }

    let page =
        fetch_page_http("/robots_protocol/robots_noindexed_and_robots_disallowed").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn robots_meta_none() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/robots_meta_none").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("none"),
        "robots meta none should be present"
    );
}

#[tokio::test]
async fn robots_meta_noodp_noydir_none_noindex() {
    if !run_live_tests() {
        return;
    }

    let page =
        fetch_page_http("/robots_protocol/robots_meta_noodp_noydir_none_noindex").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn robots_meta_multiple_tags() {
    if !run_live_tests() {
        return;
    }

    let page =
        fetch_page_http("/robots_protocol/robots_meta_multiple_tags_noindex_nofollow").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("noindex") || html.contains("nofollow"),
        "should have noindex/nofollow directives"
    );
}

#[tokio::test]
async fn meta_robots_and_x_robots_conflict() {
    if !run_live_tests() {
        return;
    }

    let page =
        fetch_page_http("/robots_protocol/meta_robots_and_x_robots_conflict").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn x_robots_multiple_directives() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/x_robots_multiple_directives").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn multiple_robots_directive_meta_tag() {
    if !run_live_tests() {
        return;
    }

    let page =
        fetch_page_http("/robots_protocol/multiple_robots_directive_meta_tag").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn multiple_googlebot_directive_meta_tag() {
    if !run_live_tests() {
        return;
    }

    let page =
        fetch_page_http("/robots_protocol/multiple_googlebot_directive_meta_tag").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn non_200_with_noindex() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/non_200_with_noindex").await;
    // May return non-200 status
    let _status = page.status_code;
}

#[tokio::test]
async fn canonicalised_with_noindex() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/canonicalised_with_noindex").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonicalised_with_non_200() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/canonicalised_with_non_200").await;
    // May return non-200 status
    let _status = page.status_code;
}

// --- Pattern matching precedence ---

#[tokio::test]
async fn allowed_same_length() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/allowed_same_length").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn allowed_shorter() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/allowed_shorter").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn allowed_longer() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/robots_protocol/allowed_longer").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- Crawl with respect_robots_txt ---

#[tokio::test]
async fn robots_crawl_with_respect_robots() {
    if !run_live_tests() {
        return;
    }

    use spider::hashbrown::HashMap;
    use spider::website::Website;

    let url = format!("{}/robots_protocol/", BASE);
    let mut website = Website::new(&url);
    website
        .with_respect_robots_txt(true)
        .with_budget(Some(HashMap::from([("*", 10)])))
        .with_depth(1)
        .with_request_timeout(Some(std::time::Duration::from_secs(30)))
        .with_crawl_timeout(Some(std::time::Duration::from_secs(60)));

    let mut rx = website.subscribe(16).expect("subscribe");
    let (done_tx, done_rx) = spider::tokio::sync::oneshot::channel();

    let collector = spider::tokio::spawn(async move {
        let mut urls = Vec::new();
        while let Ok(page) = rx.recv().await {
            urls.push(page.get_url().to_string());
        }
        let _ = done_tx.send(urls);
    });

    website.crawl_raw().await;
    website.unsubscribe();

    let urls = done_rx.await.unwrap_or_default();
    let _ = collector.await;

    if !urls.is_empty() {
        eprintln!(
            "Crawled {} pages with respect_robots_txt=true",
            urls.len()
        );
    }
}
