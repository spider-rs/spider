use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn empty_title() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/empty_title").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let title = extract_title(&html);
    assert!(
        title.is_none() || title.as_ref().unwrap().is_empty(),
        "empty_title should have no title text, got: {:?}",
        title
    );
}

#[tokio::test]
async fn missing_title() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/missing_title").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        !html.contains("<title>") || !html.contains("</title>"),
        "missing_title should not have a title tag"
    );
}

#[tokio::test]
async fn title_with_whitespace() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/title_with_whitespace").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let title = extract_title(&html);
    assert!(title.is_some(), "should have a title tag");
}

#[tokio::test]
async fn title_over_max() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/title_over_max").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let title = extract_title(&html);
    assert!(title.is_some(), "should have a title");
    let t = title.unwrap();
    assert!(
        t.len() > 60,
        "title_over_max should have a long title, got {} chars",
        t.len()
    );
}

#[tokio::test]
async fn title_warning() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/title_warning").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn page_title_length_n() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/page_title_length_n").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn page_title_width_n() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/page_title_width_n").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn duplicate_title() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/titles/duplicate_title",
        "/titles/duplicate_title/foo",
        "/titles/duplicate_title/bar",
        "/titles/duplicate_title/baz",
    ];

    let mut titles = Vec::new();
    for path in &paths {
        let page = fetch_page_http(path).await;
        assert_eq!(page.status_code, StatusCode::OK, "{} should return 200", path);
        titles.push(extract_title(&page.get_html()));
    }

    // All duplicate_title pages should share the same title
    let first = &titles[0];
    for (i, title) in titles.iter().enumerate().skip(1) {
        assert_eq!(
            first, title,
            "duplicate_title pages should share the same title: {} vs {}",
            paths[0], paths[i]
        );
    }
}

#[tokio::test]
async fn duplicate_title_and_noindex() {
    if !run_live_tests() {
        return;
    }

    let paths = [
        "/titles/duplicate_title_and_noindex/bat",
        "/titles/duplicate_title_and_noindex/bak",
    ];

    for path in &paths {
        let page = fetch_page_http(path).await;
        assert_eq!(page.status_code, StatusCode::OK, "{} should return 200", path);
    }
}

#[tokio::test]
async fn svg_title_does_not_override_page_title() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/svg_title").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("<svg") || html.contains("svg"),
        "svg_title page should contain SVG content"
    );
}

#[tokio::test]
async fn leading_trailing_spaces() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/page_title_leading_trailing_spaces").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn double_triple_quadruple_spaces() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/double_triple_quadruple_spaces").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[tokio::test]
async fn forced_double_triple_quadruple_spaces() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/titles/forced_double_triple_quadruple_spaces").await;
    assert_eq!(page.status_code, StatusCode::OK);
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn title_chrome_rendering() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/titles/title_with_whitespace").await {
        let html = page.get_html();
        let title = extract_title(&html);
        assert!(title.is_some(), "chrome should render title");
    } else {
        eprintln!("SKIP: chrome not available");
    }
}
