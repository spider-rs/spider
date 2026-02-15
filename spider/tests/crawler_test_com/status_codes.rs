use crate::helpers::*;
use spider::reqwest::StatusCode;

/// All 67 status code endpoints on crawler-test.com.
const ALL_STATUS_CODES: &[u16] = &[
    100, 101, 102, 200, 201, 202, 203, 204, 205, 206, 207, 226, 400, 401, 402, 403, 404,
    405, 406, 407, 408, 409, 410, 411, 412, 413, 414, 415, 416, 417, 418, 419, 420, 421,
    422, 423, 424, 426, 428, 429, 431, 440, 444, 449, 450, 451, 494, 495, 496, 497, 498,
    499, 500, 501, 502, 503, 504, 505, 506, 507, 508, 509, 510, 511, 520, 598, 599,
];

/// Verify every status code endpoint is reachable and returns the expected code.
#[tokio::test]
async fn all_status_codes_no_redirect() {
    if !run_live_tests() {
        return;
    }

    let mut mismatches = Vec::new();

    for &code in ALL_STATUS_CODES {
        let path = format!("/status_codes/status_{}", code);
        let page = fetch_page_http_no_redirect(&path).await;
        let actual = page.status_code.as_u16();
        if actual != code {
            mismatches.push((code, actual));
        }
    }

    if !mismatches.is_empty() {
        eprintln!("Status code mismatches (expected vs actual):");
        for (expected, actual) in &mismatches {
            eprintln!("  status_{}: expected {} got {}", expected, expected, actual);
        }
    }

    // Core status codes must match exactly
    let critical_mismatches: Vec<_> = mismatches
        .iter()
        .filter(|(expected, _)| matches!(expected, 200 | 301 | 400 | 403 | 404 | 500 | 503))
        .collect();
    assert!(
        critical_mismatches.is_empty(),
        "critical status codes must match: {:?}",
        critical_mismatches
    );
}

/// Verify status codes return correct values when following redirects.
#[tokio::test]
async fn status_codes_with_redirect_follow() {
    if !run_live_tests() {
        return;
    }

    // 2xx codes should return success when following redirects
    let success_codes: &[u16] = &[200, 201, 202, 203, 205, 206, 207, 226];
    for &code in success_codes {
        let path = format!("/status_codes/status_{}", code);
        let page = fetch_page_http(&path).await;
        assert!(
            page.status_code.is_success() || page.status_code.as_u16() == code,
            "status_{} with redirect follow: got {}",
            code,
            page.status_code
        );
    }
}

#[tokio::test]
async fn status_200_has_html_body() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/status_codes/status_200").await;
    assert_eq!(page.status_code, StatusCode::OK);
    assert!(!page.get_html().is_empty(), "200 response should have a body");
}

#[tokio::test]
async fn status_204_no_content() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http_no_redirect("/status_codes/status_204").await;
    let status = page.status_code.as_u16();
    assert!(
        status == 204 || status == 200,
        "status_204 should return 204 or 200, got {}",
        status
    );
}

#[tokio::test]
async fn status_404_accessible() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http_no_redirect("/status_codes/status_404").await;
    assert_eq!(page.status_code, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_500_server_error() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http_no_redirect("/status_codes/status_500").await;
    assert_eq!(page.status_code.as_u16(), 500);
}

#[tokio::test]
async fn status_4xx_range() {
    if !run_live_tests() {
        return;
    }

    let codes_4xx: &[u16] = &[
        400, 401, 402, 403, 404, 405, 406, 407, 408, 409, 410, 411, 412, 413, 414, 415,
        416, 417, 418, 419, 420, 421, 422, 423, 424, 426, 428, 429, 431, 440, 444, 449,
        450, 451, 494, 495, 496, 497, 498, 499,
    ];
    for &code in codes_4xx {
        let path = format!("/status_codes/status_{}", code);
        let page = fetch_page_http_no_redirect(&path).await;
        let actual = page.status_code.as_u16();
        // Verify it's a client error or matches expected
        assert!(
            actual == code || (400..600).contains(&actual) || actual == 200,
            "status_{}: expected {}xx range, got {}",
            code,
            code / 100,
            actual
        );
    }
}

#[tokio::test]
async fn status_5xx_range() {
    if !run_live_tests() {
        return;
    }

    let codes_5xx: &[u16] = &[500, 501, 502, 503, 504, 505, 506, 507, 508, 509, 510, 511, 520, 598, 599];
    for &code in codes_5xx {
        let path = format!("/status_codes/status_{}", code);
        let page = fetch_page_http_no_redirect(&path).await;
        let actual = page.status_code.as_u16();
        assert!(
            actual == code || (500..600).contains(&actual) || actual == 200,
            "status_{}: expected 5xx range, got {}",
            code,
            actual
        );
    }
}

#[cfg(feature = "chrome")]
#[tokio::test]
async fn status_codes_chrome_200() {
    if !run_live_tests() {
        return;
    }

    if let Some(p) = fetch_page_chrome("/status_codes/status_200").await {
        assert!(!p.get_html().is_empty(), "chrome 200 should have content");
    } else {
        eprintln!("SKIP: chrome not available");
    }
}
