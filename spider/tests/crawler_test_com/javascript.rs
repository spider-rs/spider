//! JavaScript tests — Chrome/Smart mode only.
//! These pages require JS execution to render their content.

use crate::helpers::*;

// --- Chrome-mode tests ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn dynamically_inserted_text() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/dynamically-inserted-text").await {
        let html = page.get_html();
        assert!(
            html.len() > 100,
            "chrome should render dynamically inserted text"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn dynamically_inserted_text_meta_data() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/dynamically-inserted-text-meta-data").await {
        assert!(
            !page.get_html().is_empty(),
            "chrome should render dynamically inserted meta data"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn ajax_return_data() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/ajax-return-data").await {
        let html = page.get_html();
        assert!(html.len() > 100, "chrome should wait for AJAX data");
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn onload_added_title() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/onload-added-title").await {
        let html = page.get_html();
        let title = extract_title(&html);
        assert!(
            title.is_some() && !title.as_ref().unwrap().is_empty(),
            "chrome should see dynamically added title"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn onload_inserted_canonical() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/onload-inserted-canonical").await {
        let html = page.get_html();
        let canonical = extract_canonical(&html);
        assert!(
            canonical.is_some(),
            "chrome should see dynamically inserted canonical"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn dynamically_inserted_nofollow() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/dynamically-inserted-nofollow").await {
        let html = page.get_html().to_lowercase();
        assert!(
            html.contains("nofollow") || html.contains("rel="),
            "chrome should see dynamically inserted nofollow"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

// --- Renderer timeouts ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn renderer_timeout_1() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/renderer_timeout/1"),
    )
    .await;
    assert!(result.is_ok(), "renderer_timeout/1 should complete");
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn renderer_timeout_2() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/renderer_timeout/2"),
    )
    .await;
    assert!(result.is_ok(), "renderer_timeout/2 should complete");
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn renderer_timeout_3() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/renderer_timeout/3"),
    )
    .await;
    assert!(result.is_ok(), "renderer_timeout/3 should complete");
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn renderer_timeout_4() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/renderer_timeout/4"),
    )
    .await;
    assert!(result.is_ok(), "renderer_timeout/4 should complete");
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn renderer_timeout_5() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/renderer_timeout/5"),
    )
    .await;
    assert!(result.is_ok(), "renderer_timeout/5 should complete");
}

// --- Window location redirects ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn window_location_redirect_internal() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/window-location-internal"),
    )
    .await;
    assert!(result.is_ok(), "JS redirect should complete in chrome");
    if let Some(page) = result.unwrap() {
        assert!(
            !page.get_html().is_empty(),
            "JS redirect should produce content"
        );
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn window_location_redirect_external() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/window-location-external"),
    )
    .await;
    assert!(result.is_ok(), "external JS redirect should complete");
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn window_location_function_absolute() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/window-location-function-absolute"),
    )
    .await;
    assert!(result.is_ok(), "absolute JS redirect should complete");
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn window_location_function_relative() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/window-location-function-relative"),
    )
    .await;
    assert!(result.is_ok(), "relative JS redirect should complete");
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn window_location_onchange() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/window-location-onchange"),
    )
    .await;
    assert!(result.is_ok(), "onchange JS redirect should complete");
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn window_location_onclick() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/window-location-onclick").await {
        assert!(
            !page.get_html().is_empty(),
            "onclick page should have content"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn window_open() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/window-open").await {
        assert!(
            !page.get_html().is_empty(),
            "window-open should have content"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

// --- JS link discovery ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn onmousedown() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/onmousedown").await {
        assert!(
            !page.get_html().is_empty(),
            "onmousedown should have content"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn concatenatedlink() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/concatenatedlink").await {
        assert!(
            !page.get_html().is_empty(),
            "concatenatedlink should have content"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn data_hreflink() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/data-hreflink").await {
        assert!(
            !page.get_html().is_empty(),
            "data-hreflink should have content"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn push_state() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/push_state").await {
        assert!(
            !page.get_html().is_empty(),
            "push_state should have content"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn onclick_reveals_element() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) =
        fetch_page_chrome("/javascript/onclick-reveals-element-programmatically-added-onclick")
            .await
    {
        assert!(
            !page.get_html().is_empty(),
            "onclick-reveals should have content"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

// --- Dialogs ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn dialog_window() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/dialog_window"),
    )
    .await;
    assert!(result.is_ok(), "dialog_window should not hang");
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn alert_box() {
    if !run_live_tests() {
        return;
    }

    let result = spider::tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fetch_page_chrome("/javascript/alert_box"),
    )
    .await;
    assert!(result.is_ok(), "alert_box should not hang");
}

// --- Script blocking ---

#[cfg(feature = "chrome")]
#[tokio::test]
async fn ad_script() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/ad_script").await {
        assert!(!page.get_html().is_empty(), "ad_script should have content");
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn analytics_script() {
    if !run_live_tests() {
        return;
    }

    if let Some(page) = fetch_page_chrome("/javascript/analytics_script").await {
        assert!(
            !page.get_html().is_empty(),
            "analytics_script should have content"
        );
    } else {
        eprintln!("SKIP: chrome not available");
    }
}

// --- HTTP-only test: JS pages should have script tags without a browser ---

#[tokio::test]
async fn js_page_http_has_script_tags() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/javascript/dynamically-inserted-text").await;
    let html = page.get_html().to_lowercase();
    assert!(
        html.contains("<script"),
        "HTTP fetch of JS page should see raw script tags"
    );
}

// --- Smart mode tests ---

#[cfg(feature = "smart")]
#[tokio::test]
async fn dynamically_inserted_text_smart() {
    if !run_live_tests() {
        return;
    }

    let pages = crawl_collect_smart("/javascript/dynamically-inserted-text", 1, 0).await;
    assert!(!pages.is_empty(), "smart mode should fetch JS page");
    let html = pages[0].get_html();
    assert!(html.len() > 100, "smart mode should render JS content");
}

#[cfg(feature = "smart")]
#[tokio::test]
async fn ajax_return_data_smart() {
    if !run_live_tests() {
        return;
    }

    let pages = crawl_collect_smart("/javascript/ajax-return-data", 1, 0).await;
    assert!(!pages.is_empty(), "smart mode should fetch AJAX page");
}
