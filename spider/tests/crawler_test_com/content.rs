use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn custom_text() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/custom_text").await;
    assert_eq!(page.status_code, StatusCode::OK);
    assert!(
        !page.get_html().is_empty(),
        "custom_text should have content"
    );
}

#[tokio::test]
async fn custom_extraction_text() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/custom_extraction_text").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn above_min_content_volume() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/above_min_content_volume").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn no_h1() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/no_h1").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        !html.contains("<h1"),
        "no_h1 page should not contain <h1> tag"
    );
}

#[tokio::test]
async fn h1_in_img() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/h1_in_img").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn mult_h1() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/mult_h1").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    let count = html.matches("<h1").count();
    assert!(
        count > 1,
        "mult_h1 page should have multiple <h1> tags, got {}",
        count
    );
}

#[tokio::test]
async fn page_html_size() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/page_html_size_n").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    assert!(!html.is_empty(), "page_html_size should have content");
}

#[tokio::test]
async fn page_content_size() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/page_content_size_n").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn word_count_100_words() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/word_count_100_words").await;
    assert_eq!(page.status_code, StatusCode::OK);
    assert!(
        !page.get_html().is_empty(),
        "word_count page should have content"
    );
}

#[tokio::test]
async fn word_count_number() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/word_count_number").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn word_count_hyphenated() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/word_count_hyphenated").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn word_count_symbols() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/word_count_symbols").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn word_count_script() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/word_count_script").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("<script"),
        "word_count_script should contain script tags"
    );
}

#[tokio::test]
async fn error_page() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/error_page").await;
    assert!(
        page.status_code.is_success()
            || page.status_code.is_client_error()
            || page.status_code.is_server_error(),
        "error_page should return some HTTP status, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn meta_content_type_text_html() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/meta_content_type_text_html").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn meta_content_type_malformed() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/meta_content_type_malformed").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn header_content_type_malformed() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/header_content_type_malformed").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn multiple_titles_and_descriptions() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/content/multiple_titles_and_descriptions").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    let title_count = html.matches("<title").count();
    assert!(title_count >= 1, "should have at least one title tag");
}

#[tokio::test]
async fn title_with_special_chars() {
    if !run_live_tests() {
        return;
    }

    let page =
        fetch_page_http("/content/title_with_newline_quote_doublequote_and_comma_characters").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let title = extract_title(&html);
    assert!(
        title.is_some(),
        "should be able to extract title with special chars"
    );
}

// --- Non-secure form fields ---

#[tokio::test]
async fn non_secure_form_fields() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/content/non_secure_form_fields_text",
        "/content/non_secure_form_fields_email",
        "/content/non_secure_form_fields_search",
        "/content/non_secure_form_fields_number",
        "/content/non_secure_form_fields_tel",
        "/content/non_secure_form_fields_url",
        "/content/non_secure_form_fields_textarea",
        "/content/non_secure_form_fields_password_and_cc",
    ];

    for path in &paths {
        let page = fetch_page_http(path).await;
        assert_eq!(
            page.status_code,
            StatusCode::OK,
            "{} should return 200",
            path
        );
    }
}

// --- Chrome ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn content_chrome_renders() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/content/custom_text").await {
        assert!(
            !page.get_html().is_empty(),
            "chrome should render content page"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}
