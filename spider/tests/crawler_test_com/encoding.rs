use crate::helpers::*;
use spider::reqwest::StatusCode;

#[tokio::test]
async fn url_with_hebrew_characters() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/encoding/url_with_foreign_characters/%D7%91%D7%9C%D7%94%D7%91%D7%9C%D7%94",
    )
    .await;
    assert!(
        page.status_code.is_success(),
        "Hebrew URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_german_characters() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/encoding/url_with_foreign_characters/Zw%C3%B6lf-gro%C3%9Fe-Boxk%C3%A4mpfer-jagen-Viktor-quer-%C3%BCber-den-Sylter-Deich",
    )
    .await;
    assert!(
        page.status_code.is_success(),
        "German URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_spanish_characters() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/encoding/url_with_foreign_characters/Fabio-me-exige-sin-tapujos-que-a%C3%B1ada-cerveza-al-whisky",
    )
    .await;
    assert!(
        page.status_code.is_success(),
        "Spanish URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_japanese_characters() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/encoding/url_with_foreign_characters/%E3%81%99%E3%81%B9%E3%81%A6%E3%81%AE%E5%8D%98%E8%AA%9E%E3%81%8C%E9%AB%98%E6%A0%A1%E7%A8%8B%E5%BA%A6%E3%81%AE%E8%BE%9E%E6%9B%B8%E3%81%AB%E8%BC%89%E3%81%A3%E3%81%A6%E3%81%84%E3%82%8B",
    )
    .await;
    assert!(
        page.status_code.is_success(),
        "Japanese URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_polish_characters() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/encoding/url_with_foreign_characters/pchn%C4%85%C4%87-w-t%C4%99-%C5%82%C3%B3d%C5%BA-je%C5%BCa-lub-o%C5%9Bm-skrzy%C5%84-fig",
    )
    .await;
    assert!(
        page.status_code.is_success(),
        "Polish URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_cyrillic_characters() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/encoding/url_with_foreign_characters/%D0%A8%D0%B5%D1%84-%D0%B2%D0%B7%D1%8A%D1%8F%D1%80%D1%91%D0%BD-%D1%82%D1%87%D0%BA-%D1%89%D0%B8%D0%BF%D1%86%D1%8B-%D1%81-%D1%8D%D1%85%D0%BE%D0%BC-%D0%B3%D1%83%D0%B4%D0%B1%D0%B0%D0%B9-%D0%96%D1%8E%D0%BB%D1%8C",
    )
    .await;
    assert!(
        page.status_code.is_success(),
        "Cyrillic URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_arabic_characters() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/encoding/url_with_foreign_characters/%EF%B4%BF%D9%85%D8%AD%D9%85%D8%AF-%D8%B1%D8%B3%D9%88%D9%84-%D8%A7%D9%84%D9%84%D9%87-%D9%88%D8%A7%D9%84%D8%B0%D9%8A%D9%86-%D9%85%D8%B9%D9%87-%D8%A3%D8%B4%D8%AF%D8%A7%D8%A1",
    )
    .await;
    assert!(
        page.status_code.is_success(),
        "Arabic URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_greek_characters() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/encoding/url_with_foreign_characters/%CE%B3%CF%81%CE%AC%CE%BC%CE%BC%CE%B1%CF%84%CE%B1-%CF%84%CE%BF%CF%85-%CE%B9%CF%83%CF%80%CE%B1%CE%BD%CE%B9%CE%BA%CE%BF%CF%8D-%CE%B1%CE%BB%CF%86%CE%B1%CE%B2%CE%AE%CF%84%CE%BF%CF%85-%CE%BA%CE%B1%CE%B8%CF%8E%CF%82",
    )
    .await;
    assert!(
        page.status_code.is_success(),
        "Greek URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn url_with_scandinavian_characters() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/encoding/url_with_foreign_characters/as%C3%B8d-%C3%A6ada-%C3%A5djghf-g%C3%A4gfd-as%C3%B6dsads",
    )
    .await;
    assert!(
        page.status_code.is_success(),
        "Scandinavian URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn double_encoded_url() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http(
        "/encoding/double_encoded_url/Zw%25C3%25B6lf-gro%25C3%259Fe-Boxk%25C3%25A4mpfer-jagen-Viktor-quer-%25C3%25BCber-den-Sylter-Deich",
    )
    .await;
    assert!(
        page.status_code.is_success(),
        "double encoded URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn inconsistent_character_encoding() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/encoding/inconsistent_character_encoding").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let _html = page.get_html();
}

#[tokio::test]
async fn encoded_hashbang() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/encoding/url/encoded_hashbang%23abc").await;
    assert!(
        page.status_code.is_success(),
        "encoded hashbang URL should resolve, got {}",
        page.status_code
    );
}

#[tokio::test]
async fn page_titles_character_encoded() {
    if !run_live_tests() {
        return;
    }

    let page = fetch_page_http("/encoding/page_titles_character_encoded").await;
    assert_eq!(page.status_code, StatusCode::OK);
    let html = page.get_html();
    let title = extract_title(&html);
    assert!(
        title.is_some(),
        "should extract title even with character encoding"
    );
}
