use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn canonical_tag_basic() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_tag").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let canonical = extract_canonical(&html);
    assert!(canonical.is_some(), "page should have a canonical tag");
}

/// All numbered canonical tag variants (2-25) should be accessible and have canonical tags.
#[tokio::test]
async fn canonical_tag_numbered_variants() {
    if !run_live_tests() {
        return;
    }

    for n in 2..=25 {
        let path = format!("/canonical_tags/canonical_tag/{}", n);
        let page = fetch_page_http(&path).await;
        assert_eq!(
            page.status_code,
            StatusCode::OK,
            "canonical_tag/{} should return 200",
            n
        );
        let html = page.get_html();
        let canonical = extract_canonical(&html);
        assert!(
            canonical.is_some(),
            "canonical_tag/{} should have a canonical tag",
            n
        );
    }
}

#[tokio::test]
async fn canonical_tag_like_page() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_tag_like_page").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_duplicate_description() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_duplicate_description").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_tag_in_header() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_tag_in_header").await;
    assert_eq!(page.status_code, StatusCode::OK);
    if let Some(headers) = &page.headers {
        let has_link_header = headers.get("link").is_some();
        assert!(
            has_link_header,
            "canonical_tag_in_header should have a Link HTTP header"
        );
    }
}

#[tokio::test]
async fn canonical_tag_uppercase() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_tag_uppercase").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let canonical = extract_canonical(&html);
    assert!(
        canonical.is_some(),
        "should parse canonical tag even with uppercase"
    );
}

#[tokio::test]
async fn relative_root_canonical_tag() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/relative_root_canonical_tag").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn relative_canonical_tag() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/relative_canonical_tag").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let canonical = extract_canonical(&html);
    assert!(canonical.is_some(), "should have a relative canonical tag");
    let c = canonical.unwrap();
    assert!(
        !c.starts_with("http") || c.contains("crawler-test.com"),
        "relative canonical: {}",
        c
    );
}

#[tokio::test]
async fn canonical_tag_outside_head() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_tag_outside_head").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_html_header_conflict() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_tag_html_header_conflict").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let html_canonical = extract_canonical(&html);
    assert!(html_canonical.is_some(), "should have HTML canonical tag");
}

#[tokio::test]
async fn canonical_html_conflict() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_tag_html_conflict").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn page_with_external_canonical() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/page_with_external_canonical").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let canonical = extract_canonical(&html);
    assert!(canonical.is_some(), "should have canonical tag");
}

#[tokio::test]
async fn page_without_canonical() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/page_without_canonical_tag").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let canonical = extract_canonical(&html);
    assert!(
        canonical.is_none(),
        "page_without_canonical_tag should have no canonical"
    );
}

#[tokio::test]
async fn unlinked_canonical() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/unlinked_canonical").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn unlinked_canonical_header() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/unlinked_canonical_header").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_tag_og_url_conflict() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_tag_og_url_conflict").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonicalised_to_disallowed_url() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonicalised_to_disallowed_url").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_with_self_reference() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_tag_with_self_reference").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let canonical = extract_canonical(&html);
    assert!(canonical.is_some(), "should have self-referencing canonical");
    let c = canonical.unwrap();
    assert!(
        c.contains("canonical_tag_with_self_reference"),
        "self-referencing canonical should point to itself: {}",
        c
    );
}

// --- Non-head canonical ---

#[tokio::test]
async fn non_head_canonical() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/non_head_canonical").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn non_head_canonical_link() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/non_head_canonical_link").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn non_head_canonical_link_2() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/non_head_canonical_link_2").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- Port variants ---

#[tokio::test]
async fn canonical_port_80() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_port_80").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_port_443() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_port_443").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_port_8080() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_port_8080").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- URL encoding ---

#[tokio::test]
async fn canonical_url_encoded_vs_non_encoded() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/canonical_tags/canonical_url_encoded_vs_non_encoded/caf%C3%A9",
    )
    .await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_url_encoded_emoji() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/canonical_tags/canonical_url_encoded_vs_non_encoded/%F0%9F%8D%BA/ist",
    )
    .await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- Parameter / case sensitivity ---

#[tokio::test]
async fn canonical_parameter_key_case_sensitive() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/canonical_tags/canonical_prameter_key_is_case_sensitive?key=value",
    )
    .await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_parameter_value_case_sensitive() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/canonical_tags/canonical_prameter_value_is_case_sensitive?key=value",
    )
    .await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_url_fragments() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_url_fragments").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_parameter_order() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/canonical_tags/canonical_different_parameter_order?key2=value2&key=value",
    )
    .await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_hostname_case_insensitive() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_hostname_case_insensitive").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_protocol_case_insensitive() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_protocol_case_insensitive").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_path_is_case_sensitive() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_path_is_case_sensitive").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_url_with_slash() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_url_with_slash").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn canonical_trailing_dot() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/canonical_tags/canonical_trailing_dot").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

// --- Chrome ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn canonical_tag_chrome() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/canonical_tags/canonical_tag").await {
        let html = page.get_html();
        let canonical = extract_canonical(&html);
        assert!(canonical.is_some(), "chrome should see canonical tag");
    } else {
        eprintln!("SKIP: chrome not available");
    }
}
