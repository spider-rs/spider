use aho_corasick::{AhoCorasick, AhoCorasickBuilder};

#[cfg(all(feature = "chrome", feature = "real_browser"))]
use chromiumoxide::{
    cdp::js_protocol::runtime::{CallFunctionOnParams, EvaluateParams},
    error::CdpError,
    layout::Point,
    Page,
};
use std::time::Duration;

#[cfg(all(feature = "chrome", feature = "real_browser"))]
use crate::utils::{page_wait, perform_smart_mouse_movement, RequestError, CF_WAIT_FOR};
#[cfg(all(feature = "chrome", feature = "real_browser"))]
use base64::prelude::*;

static VERIFY_PATTERNS: &[&[u8]] = &[
    b"verifying you are human",
    b"review the security of your connection",
    b"please verify you are a human",
    b"checking your browser before accessing",
    b"prove you are human",
    b"checking if the site connection is secure",
];

/// Imperva iframe patterns.
static IMPERVA_IFRAME_PATTERNS: &[&[u8]] = &[
    b"geo.captcha-delivery.com",
    b"captcha-delivery.com",
    b"Verification system",
    b"Verification Required",
    b"Verification successful",
    b"Verifying device",
];

/// Recaptcha iframe patterns.
static RECAPTCHA_PATTERNS: &[&[u8]] = &[
    b"https://www.google.com/recaptcha/",
    b"/recaptcha/",
    b"recaptcha/api2/anchor",
    b"recaptcha/enterprise/bframe",
];

/// Geetest patterns.
static GEETEST_PATTERNS: &[&[u8]] = &[
    b"id=\"embed-captcha\"",
    b"class=\"gee-test",
    b"class=\"gee-test__placeholder",
];

/// Geetest loading patterns.
static GEETEST_LOADING_PATTERNS: &[&[u8]] = &[b"Loading GeeTest", b"geetest_wait", b"geetest_init"];

/// Geetest visible patterns.
static GEETEST_VISIBLE_PATTERNS: &[&[u8]] = &[
    b"geetest_widget",
    b"geetest_slider_button",
    b"geetest_canvas",
    b"geetest_canvas_slice",
];

/// Imperva wait patterns.
static IMPERVA_WAIT_PATTERNS: &[&[u8]] = &[
    b"Verifying the device",
    b"Verifying the device...",
    b"The requested content will be available after verification",
    b"available after verification",
];

/// Imperva iframe phase patterns.
static IMPERVA_IFRAME_PHASE_PATTERNS: &[&[u8]] = &[
    b"geo.captcha-delivery.com",
    b"captcha-delivery.com",
    b"Verification system",
];

/// Hcaptcha wait patterns.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
static HCAPTCHA_IFRAME_PATTERNS: &[&[u8]] = &[
    b"newassets.hcaptcha.com",
    b"hcaptcha.com/captcha",
    b"Widget containing checkbox for hCaptcha",
    b"data-hcaptcha-widget-id",
];

/// RC enterprise guards.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
static RC_ENTERPRISE_GUARD_PATTERNS: &[&[u8]] = &[
    b"__recaptcha_api",
    b"/recaptcha/enterprise/",
    b"rc-imageselect",
    b"rc-imageselect-tile",
];

static LEMIN_PATTERNS: &[&[u8]] = &[b"id=\"lemin-cropped-captcha\""];

/// RC verify btn patterns.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
static RC_VERIFY_BUTTON_PATTERNS: &[&[u8]] = &[b"id=\"recaptcha-verify-button\"", b">Verify<"];

/// RC tile patterns.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
static RC_TILE_CLASS_PATTERNS: &[&[u8]] = &[b"rc-imageselect-tile"];

#[cfg(all(feature = "chrome", feature = "real_browser"))]
lazy_static! {
    /// hCaptcha‑iframe matcher (used inside Imperva flow)
    static ref HCAPTCHA_IFRAME_AC: AhoCorasick = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build(HCAPTCHA_IFRAME_PATTERNS)
        .expect("valid hCaptcha iframe patterns");

            static ref RC_ENTERPRISE_GUARD_AC: AhoCorasick = AhoCorasickBuilder::new()
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .ascii_case_insensitive(false)
        .build(RC_ENTERPRISE_GUARD_PATTERNS)
        .expect("valid enterprise‑recaptcha guard patterns");

    // Verify‑button detection (either the hidden button id or the visible “Verify” text).
    static ref RC_VERIFY_BUTTON_AC: AhoCorasick = AhoCorasickBuilder::new()
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .ascii_case_insensitive(false)
        .build(RC_VERIFY_BUTTON_PATTERNS)
        .expect("valid verify‑button patterns");

    // Tile‑class matcher – used to locate every tile in the HTML.
    static ref RC_TILE_CLASS_AC: AhoCorasick = AhoCorasickBuilder::new()
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .ascii_case_insensitive(false)
        .build(RC_TILE_CLASS_PATTERNS)
        .expect("valid tile‑class pattern");
}

#[cfg(any(not(feature = "wreq"), feature = "cache_request"))]
lazy_static! {
    /// Gemini client – plain reqwest client (no middleware).
    static ref GEMINI_CLIENT: reqwest::Client = {
        reqwest::ClientBuilder::new()
            .timeout(Duration::from_millis(20_000))
            .build()
            .expect("failed to build Gemini client")
    };
}
#[cfg(all(feature = "wreq", not(feature = "cache_request")))]
lazy_static! {
    /// Gemini client – plain wreq client (no middleware).
    static ref GEMINI_CLIENT: wreq::Client = {
        wreq::ClientBuilder::new()
            .timeout(Duration::from_millis(20_000))
            .build()
            .expect("failed to build Gemini client")
    };
}

lazy_static! {
    /// Imperva check
    static ref AC_IMPERVA_IFRAME: aho_corasick::AhoCorasick = aho_corasick::AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build(IMPERVA_IFRAME_PATTERNS)
        .expect("valid imperva iframe patterns");
    /// Bot verify.
    static ref AC: AhoCorasick =  aho_corasick::AhoCorasickBuilder::new()
        .match_kind(aho_corasick::MatchKind::LeftmostLongest)
        .build(VERIFY_PATTERNS)
        .unwrap();
    /// Recaptcha patterns.
    static ref RECAPTCHA_AC: AhoCorasick = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build(RECAPTCHA_PATTERNS)
        .expect("valid recaptcha patterns");
    /// GeeTest patterns.
    static ref GEETEST_AC: AhoCorasick = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build(GEETEST_PATTERNS)
        .expect("valid geetest patterns");
    /// GeeTest loading AC.
    static ref GEETEST_LOADING_AC: AhoCorasick = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build(GEETEST_LOADING_PATTERNS)
        .expect("valid geetest loading patterns");
    /// GeeTest visible‑challenge matcher
    static ref GEETEST_VISIBLE_AC: AhoCorasick = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build(GEETEST_VISIBLE_PATTERNS)
        .expect("valid geetest visible patterns");
    /// Imperva wait AC.
    static ref IMPERVA_WAIT_AC: AhoCorasick = AhoCorasickBuilder::new()
            .ascii_case_insensitive(true)
            .match_kind(aho_corasick::MatchKind::LeftmostFirst)
            .build(IMPERVA_WAIT_PATTERNS)
            .expect("valid Imperva wait‑screen patterns");

    /// Imperva iframe matcher.
    static ref IMPERVA_IFRAME_PHASE_AC: AhoCorasick = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build(IMPERVA_IFRAME_PHASE_PATTERNS)
        .expect("valid Imperva iframe‑phase patterns");
    /// Lemin match.
    static ref LEMIN_AC: AhoCorasick = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build(LEMIN_PATTERNS)
        .expect("valid lemin patterns");
}

#[cfg(feature = "chrome")]
/// CF prefix scan bytes.
const CF_PREFIX_SCAN_BYTES: usize = 120;

#[cfg(feature = "chrome")]
#[inline(always)]
/// CF slice prefix.
fn cf_prefix_slice(b: &[u8]) -> &[u8] {
    if b.len() > CF_PREFIX_SCAN_BYTES {
        &b[..CF_PREFIX_SCAN_BYTES]
    } else {
        b
    }
}

#[cfg(feature = "chrome")]
lazy_static! {
    /// CF end match.
    static ref CF_END: &'static [u8; 62] =
        b"target=\"_blank\">Cloudflare</a></div></div></div></body></html>";
    /// CF end second template.
    static ref CF_END2: &'static [u8; 72] =
        b"Performance &amp; security by Cloudflare</div></div></div></body></html>";
    /// CF head.
    static ref CF_HEAD: &'static [u8; 34] = b"<html><head>\n    <style global=\"\">";
    /// CF mock frame.
    static ref CF_MOCK_FRAME: &'static [u8; 137] = b"<iframe height=\"1\" width=\"1\" style=\"position: absolute; top: 0px; left: 0px; border: none; visibility: hidden;\"></iframe>\n\n</body></html>";
    /// Cf just a moment.
    static ref CF_JUST_A_MOMENT: &'static [u8] =
        b"<!DOCTYPE html><html lang=\"en-US\" dir=\"ltr\"><head><title>Just a moment...</title>";

    // Fast prefix-only matcher (scan only the first ~120 bytes).
    static ref CF_JUST_A_MOMENT_AC: aho_corasick::AhoCorasick = aho_corasick::AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .build([
            b"<title>Just a moment...</title>".as_slice(),
            b"Just a moment...".as_slice(),
        ])
        .expect("valid CF just-a-moment patterns");
}

#[inline(always)]
/// Detect recaptcha.
pub fn detect_recaptcha(html: &[u8]) -> bool {
    RECAPTCHA_AC.is_match(html)
}

#[inline(always)]
/// Detect GeeTest.
pub fn detect_geetest(html: &[u8]) -> bool {
    GEETEST_AC.is_match(html)
}

#[inline(always)]
/// Detect lemin.
pub fn detect_lemin(html: &[u8]) -> bool {
    LEMIN_AC.is_match(html)
}

/// Looks like GeeTest.
#[inline(always)]
pub fn looks_like_geetest(html: &[u8]) -> bool {
    GEETEST_AC.is_match(html)
}

/// Looks like GeeTest loading.
#[inline(always)]
pub fn looks_like_geetest_loading(html: &[u8]) -> bool {
    GEETEST_LOADING_AC.is_match(html)
}

/// Geetest challenge visible.
#[inline(always)]
pub fn looks_like_geetest_challenge_visible(html: &[u8]) -> bool {
    GEETEST_VISIBLE_AC.is_match(html)
}

#[inline(always)]
/// Imperva challenge size
pub fn imperva_challenge_sized(len: usize) -> bool {
    len > 0 && len <= 220_000
}

#[inline(always)]
/// Looks like imperva wait screen.
pub fn looks_like_imperva_wait_screen(html: &[u8]) -> bool {
    imperva_challenge_sized(html.len()) && IMPERVA_WAIT_AC.is_match(html)
}

#[inline(always)]
/// Looks like imperva phase screen.
pub fn looks_like_imperva_iframe_phase(html: &[u8]) -> bool {
    imperva_challenge_sized(html.len()) && IMPERVA_IFRAME_PHASE_AC.is_match(html)
}

#[inline(always)]
#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// Looks like hcaptcha iframe.
pub fn looks_like_hcaptcha_iframe(html: &[u8]) -> bool {
    imperva_challenge_sized(html.len()) && HCAPTCHA_IFRAME_AC.is_match(html)
}

#[inline(always)]
#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// Looks like imperva.
pub fn looks_like_imperva_any(html: &[u8]) -> bool {
    looks_like_imperva_wait_screen(html)
        || looks_like_imperva_iframe_phase(html)
        || looks_like_hcaptcha_iframe(html)
}

#[cfg(feature = "chrome")]
#[inline]
/// Is turnstile page?
pub(crate) fn detect_cf_turnstyle(b: &[u8]) -> bool {
    if b.ends_with(CF_END.as_ref()) || b.ends_with(CF_END2.as_ref()) {
        return true;
    }

    if b.starts_with(CF_HEAD.as_ref()) && b.ends_with(CF_MOCK_FRAME.as_ref()) {
        return true;
    }

    let pfx = cf_prefix_slice(b);

    pfx.starts_with(CF_JUST_A_MOMENT.as_ref()) || CF_JUST_A_MOMENT_AC.is_match(pfx)
}

lazy_static! {
    /// Apache server forbidden.
    pub static ref APACHE_FORBIDDEN: &'static [u8; 317] = br#"<!DOCTYPE HTML PUBLIC "-//IETF//DTD HTML 2.0//EN">
<html><head>
<title>403 Forbidden</title>
</head><body>
<h1>Forbidden</h1>
<p>You don't have permission to access this resource.</p>
<p>Additionally, a 403 Forbidden
error was encountered while trying to use an ErrorDocument to handle the request.</p>
</body></html>"#;

    /// Open Resty forbidden.
    pub static ref OPEN_RESTY_FORBIDDEN: &'static [u8; 125] = br#"<html><head><title>403 Forbidden</title></head>
<body>
<center><h1>403 Forbidden</h1></center>
<hr><center>openresty</center>"#;


  /// Empty html.
  pub static ref EMPTY_HTML_BASIC: &'static [u8; 13] = b"<html></html>";
  /// The vision endpoint gemini.
  pub static ref GEMINI_VISION_ENDPOINT: String = {
    std::env::var("GEMINI_VISION_ENDPOINT").unwrap_or("https://generativelanguage.googleapis.com/v1beta/models/gemini-pro-vision".into())
  };
}

#[inline(always)]
/// Detect imperva verification iframe.
pub fn detect_imperva_verification_iframe(html: &[u8]) -> bool {
    AC_IMPERVA_IFRAME.is_match(html)
}

/// A combined “looks like Imperva verification page” check.
/// Use this before deciding that X-Cdn: Imperva implies Imperva.
#[inline(always)]
pub fn looks_like_imperva_verify(content_len: usize, html: &[u8]) -> bool {
    imperva_challenge_sized(content_len) && detect_imperva_verification_iframe(html)
}

/// Detect if openresty hard 403 is forbidden and should not retry.
#[inline(always)]
pub fn detect_open_resty_forbidden(b: &[u8]) -> bool {
    b.starts_with(*OPEN_RESTY_FORBIDDEN)
}

/// Detect if a page is forbidden and should not retry.
#[inline(always)]
pub fn detect_hard_forbidden_content(b: &[u8]) -> bool {
    b == *APACHE_FORBIDDEN || detect_open_resty_forbidden(b)
}

/// Needs bot verification.
#[inline(always)]
pub fn contains_verification(text: &Vec<u8>) -> bool {
    AC.is_match(text)
}

/// Handle protected pages via chrome. This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
#[inline(always)]
pub async fn cf_handle(
    b: &mut Vec<u8>,
    page: &chromiumoxide::Page,
    target_url: &str,
    viewport: &Option<crate::configuration::Viewport>,
) -> Result<bool, chromiumoxide::error::CdpError> {
    let mut validated = false;

    let page_result = tokio::time::timeout(tokio::time::Duration::from_secs(30), async {
        let page_navigate = async {
            // force upgrade https check.
            if let Some(page_url) = page.url().await? {
                if page_url == "about:blank" {
                    let target_url = if target_url.starts_with("http://") {
                        let mut s = String::with_capacity(target_url.len() + 1);
                        s.push_str("https://");
                        s.push_str(&target_url["http://".len()..]);
                        s
                    } else {
                        target_url.to_string()
                    };
                    let _ = page.goto(target_url).await?.wait_for_navigation().await?;
                }
                else if page_url.starts_with("http://") {
                    let _ = page.goto(page_url.replacen("http://", "https://", 1)).await?;
                } else {
                    tokio::time::sleep(Duration::from_millis(3_500)).await;
                }
            }

            Ok::<(), chromiumoxide::error::CdpError>(())
        };

        // get the csp settings before hand
        let _ = tokio::join!(page.disable_network_cache(true), page_navigate, perform_smart_mouse_movement(page, viewport));

        for _ in 0..10 {
            let mut wait_for = CF_WAIT_FOR.clone();

            let mut clicks = 0usize;
            let mut hidden = false;

            if let Ok(els) = page
                .find_elements_pierced(
                    r#"
                div[id*="turnstile"],
                iframe[src*="challenges.cloudflare.com"],
                iframe[src*="turnstile"],
                iframe[title*="widget"],
                input[type="checkbox"]"#,
                )
                .await
            {
                perform_smart_mouse_movement(page, viewport).await;
                for el in els {
                    let f = async {
                        match el.clickable_point().await {
                            Ok(pt) => page.click(pt).await.is_ok() || el.click().await.is_ok(),
                            Err(_) => el.click().await.is_ok(),
                        }
                    };

                    let (did_click, _) =
                        tokio::join!(f, perform_smart_mouse_movement(page, viewport));

                    if did_click {
                        clicks += 1;
                    }
                }
            } else {

                hidden = true;
                let wait = Some(wait_for.clone());
                let _ = tokio::join!(
                    page_wait(page, &wait),
                    perform_smart_mouse_movement(page, viewport)
                );
            }

            if !hidden && clicks == 0 {
                let f = page.evaluate(
                    r#"document.querySelectorAll("iframe,input")?.forEach(el => el.click());document.querySelector('.cf-turnstile')?.click();"#,
                );
                let _ = tokio::join!(f, perform_smart_mouse_movement(page, viewport));
            }

            wait_for.page_navigations = true;
            let wait = Some(wait_for.clone());

            let _ = tokio::join!(
                page_wait(page, &wait),
                perform_smart_mouse_movement(page, viewport),
            );

            if let Ok(mut next_content) = page.outer_html_bytes().await {
                if !detect_cf_turnstyle(&next_content) {
                    validated = true;
                    wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                        core::time::Duration::from_secs(4),
                    ))
                    .into();
                    page_wait(page, &Some(wait_for)).await;
                    if let Ok(nc) = page.outer_html_bytes().await {
                        next_content = nc;
                    }
                } else if contains_verification(&next_content) {
                    wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                        core::time::Duration::from_millis(3500),
                    ))
                    .into();
                    page_wait(page, &Some(wait_for.clone())).await;

                    if let Ok(nc) = page.outer_html_bytes().await {
                        next_content = nc;
                    }
                    if !detect_cf_turnstyle(&next_content) {
                        validated = true;
                        page_wait(page, &Some(wait_for)).await;
                        if let Ok(nc) = page.outer_html_bytes().await {
                            next_content = nc;
                        }
                    }
                };

                *b = next_content;

                if validated {
                    break;
                }
            }
        }

        Ok::<(), chromiumoxide::error::CdpError>(())
    })
    .await;

    match page_result {
        Ok(_) => Ok(validated),
        _ => Err(chromiumoxide::error::CdpError::Timeout),
    }
}

/// Handle imperva protected pages via chrome. This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
#[inline(always)]
pub async fn imperva_handle(
    b: &mut Vec<u8>,
    page: &chromiumoxide::Page,
    _target_url: &str,
    viewport: &Option<crate::configuration::Viewport>,
) -> Result<bool, chromiumoxide::error::CdpError> {
    // -----------------------------------------------------------------
    // Fast‑path – bail out early if the response does not look like an
    // Imperva challenge at all.
    // -----------------------------------------------------------------
    if !looks_like_imperva_any(b.as_slice()) {
        return Ok(false);
    }

    // -----------------------------------------------------------------
    // Drag‑helpers (unchanged)
    // -----------------------------------------------------------------
    #[inline(always)]
    fn pt(x: f64, y: f64) -> chromiumoxide::layout::Point {
        chromiumoxide::layout::Point { x, y }
    }

    #[inline(always)]
    fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
        if v < lo {
            lo
        } else if v > hi {
            hi
        } else {
            v
        }
    }

    /// Build js drag handler.
    #[inline(always)]
    fn build_js_drag(fx: f64, fy: f64, tx: f64, ty: f64) -> String {
        use core::fmt::Write as _;
        let mut s = String::with_capacity(1024);
        let _ = write!(
            &mut s,
            r#"(function(){{const fx={:.3},fy={:.3},tx={:.3},ty={:.3};
const at=(x,y)=>document.elementFromPoint(x,y);
const fire=(el,type,x,y)=>{{if(!el)return;const o={{bubbles:true,cancelable:true,clientX:x,clientY:y,buttons:1}};el.dispatchEvent(new MouseEvent(type,o));try{{const p=type==='mousedown'?'pointerdown':type==='mousemove'?'pointermove':type==='mouseup'?'pointerup':type;el.dispatchEvent(new PointerEvent(p,{{bubbles:true,cancelable:true,clientX:x,clientY:y,buttons:1,pointerId:1,isPrimary:true}}));}}catch(e){{}}}};
const el0=at(fx,fy);fire(el0,'mousedown',fx,fy);
for(let i=1;i<=18;i++){{const t=i/18,x=fx+(tx-fx)*t,y=fy+(ty-fy)*t;fire(at(x,y)||el0,'mousemove',x,y);}}
fire(at(tx,ty)||el0,'mouseup',tx,ty);return true;}})()"#,
            fx, fy, tx, ty
        );
        s
    }

    // -----------------------------------------------------------------
    // Main solving loop (unchanged apart from the matcher calls)
    // -----------------------------------------------------------------
    let mut validated = false;

    let page_result = tokio::time::timeout(tokio::time::Duration::from_secs(30), async {
        // Disable cache + a little mouse‑movement jitter.
        let _ = tokio::join!(
            page.disable_network_cache(true),
            perform_smart_mouse_movement(page, viewport)
        );

        for _ in 0..10 {
            let mut wait_for = CF_WAIT_FOR.clone();

            // ---------------------------------------------------------
            // Pull HTML once per iteration.
            // ---------------------------------------------------------
            let cur_html = match page.outer_html_bytes().await {
                Ok(h) => h,
                Err(_) => {
                    let wait = Some(wait_for.clone());
                    let _ = tokio::join!(
                        page_wait(page, &wait),
                        perform_smart_mouse_movement(page, viewport),
                    );
                    continue;
                }
            };
            *b = cur_html;

            // ---------------------------------------------------------
            // If we have left the challenge, we are done.
            // ---------------------------------------------------------
            if !looks_like_imperva_any(b.as_slice()) {
                validated = true;
                break;
            }

            // ---------------------------------------------------------
            // 0️⃣  hCaptcha checkbox flow (used inside Imperva pages)
            // ---------------------------------------------------------
            let hcaptcha_iframe_present = page
                .find_elements_pierced(
                    r#"iframe[src*="hcaptcha.com"], iframe[src*="newassets.hcaptcha.com"]"#,
                )
                .await
                .map(|els| !els.is_empty())
                .unwrap_or(false);

            if hcaptcha_iframe_present || looks_like_hcaptcha_iframe(b.as_slice()) {
                if let Ok(boxes) = page.find_elements_pierced(r#"#checkbox"#).await {
                    if let Some(cb_el) = boxes.into_iter().next() {
                        // Click the checkbox – prefer the clickable point.
                        let clicked = match cb_el.clickable_point().await {
                            Ok(p) => page.click(p).await.is_ok() || cb_el.click().await.is_ok(),
                            Err(_) => cb_el.click().await.is_ok(),
                        };

                        if clicked {
                            // Give the page a moment to render/transition.
                            wait_for.delay = crate::features::chrome_common::WaitForDelay::new(
                                Some(core::time::Duration::from_millis(900)),
                            )
                            .into();
                            wait_for.idle_network =
                                crate::features::chrome_common::WaitForIdleNetwork::new(
                                    core::time::Duration::from_secs(6).into(),
                                )
                                .into();
                            wait_for.page_navigations = true;

                            let wait = Some(wait_for.clone());
                            let _ = tokio::join!(
                                page_wait(page, &wait),
                                perform_smart_mouse_movement(page, viewport),
                            );

                            if let Ok(nc) = page.outer_html_bytes().await {
                                *b = nc;
                                if !looks_like_imperva_any(b.as_slice()) {
                                    validated = true;
                                    break;
                                }
                            }
                        } else {
                            // Click failed – wait a bit and retry.
                            wait_for.delay = crate::features::chrome_common::WaitForDelay::new(
                                Some(core::time::Duration::from_millis(650)),
                            )
                            .into();
                            let wait = Some(wait_for.clone());
                            let _ = tokio::join!(
                                page_wait(page, &wait),
                                perform_smart_mouse_movement(page, viewport),
                            );
                        }

                        // Continue the outer loop – we may now be in the slider phase.
                        continue;
                    }
                }

                // No checkbox yet – behave like Cloudflare: just wait for the iframe to load.
                wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                    core::time::Duration::from_millis(900),
                ))
                .into();
                wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                    core::time::Duration::from_secs(6).into(),
                )
                .into();
                wait_for.page_navigations = true;

                let wait = Some(wait_for.clone());
                let _ = tokio::join!(
                    page_wait(page, &wait),
                    perform_smart_mouse_movement(page, viewport),
                );
                if let Ok(nc) = page.outer_html_bytes().await {
                    *b = nc;
                    if !looks_like_imperva_any(b.as_slice()) {
                        validated = true;
                        break;
                    }
                }
                continue;
            }

            // ---------------------------------------------------------
            // 1️⃣  WAIT SCREEN – just a static “please wait” page.
            // ---------------------------------------------------------
            if looks_like_imperva_wait_screen(b.as_slice()) {
                wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                    core::time::Duration::from_millis(1_100),
                ))
                .into();
                wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                    core::time::Duration::from_secs(7).into(),
                )
                .into();
                wait_for.page_navigations = true;

                let wait = Some(wait_for.clone());
                let _ = tokio::join!(
                    page_wait(page, &wait),
                    perform_smart_mouse_movement(page, viewport),
                );

                if let Ok(nc) = page.outer_html_bytes().await {
                    *b = nc;
                }
                continue;
            }

            // ---------------------------------------------------------
            // 2️⃣  Imperva iframe / slider phase.
            // ---------------------------------------------------------
            let verify_iframe_present = page
                .find_elements_pierced(
                    r#"
                    iframe[src*="geo.captcha-delivery.com"],
                    iframe[src*="captcha-delivery.com"],
                    iframe[title*="Verification system"],
                    iframe[title*="verification system"]
                    "#,
                )
                .await
                .map(|els| !els.is_empty())
                .unwrap_or(false);

            if verify_iframe_present || looks_like_imperva_iframe_phase(b.as_slice()) {
                let mut did_drag = false;

                // -----------------------------------------------------------------
                // Try to locate a native slider handle first.
                // -----------------------------------------------------------------
                if let Ok(handles) = page
                    .find_elements_pierced(
                        r#"
                        .slider,
                        [class*="sliderHandle"],
                        [class*="slider-handle"],
                        [class*="slider"]
                        "#,
                    )
                    .await
                {
                    if let Some(handle) = handles.into_iter().next() {
                        if let (Ok(hb), Ok(conts)) = (
                            handle.bounding_box().await,
                            page.find_elements_pierced(
                                r#"
                                .sliderContainer,
                                [class*="sliderContainer"],
                                .slider-container,
                                [class*="slider-container"]
                                "#,
                            )
                            .await,
                        ) {
                            if let Some(container) = conts.into_iter().next() {
                                if let Ok(cb) = container.bounding_box().await {
                                    let from = pt(hb.x + hb.width * 0.5, hb.y + hb.height * 0.5);
                                    let to_x = cb.x + cb.width - 8.0;
                                    let to_y = cb.y + cb.height * 0.5;
                                    let to = pt(
                                        clamp(to_x, cb.x + 2.0, cb.x + cb.width - 2.0),
                                        clamp(to_y, cb.y + 2.0, cb.y + cb.height - 2.0),
                                    );

                                    // Small mouse‑move jitter before the drag.
                                    let _ = tokio::join!(
                                        perform_smart_mouse_movement(page, viewport),
                                        async {
                                            let _ = page.move_mouse(from).await;
                                        }
                                    );

                                    if page.click_and_drag(from, to).await.is_ok() {
                                        did_drag = true;
                                    }
                                }
                            }
                        }
                    }
                }

                // -----------------------------------------------------------------
                // Fallback – build a JS drag that uses the container bbox.
                // -----------------------------------------------------------------
                if !did_drag {
                    if let Ok(conts) = page
                        .find_elements_pierced(
                            r#"
                            .sliderContainer,
                            [class*="sliderContainer"],
                            .slider-container,
                            [class*="slider-container"]
                            "#,
                        )
                        .await
                    {
                        if let Some(container) = conts.into_iter().next() {
                            if let Ok(cb) = container.bounding_box().await {
                                let from_x = clamp(cb.x + 10.0, cb.x + 2.0, cb.x + cb.width - 2.0);
                                let from_y = clamp(
                                    cb.y + cb.height * 0.5,
                                    cb.y + 2.0,
                                    cb.y + cb.height - 2.0,
                                );
                                let to_x = clamp(
                                    cb.x + cb.width - 10.0,
                                    cb.x + 2.0,
                                    cb.x + cb.width - 2.0,
                                );
                                let to_y = from_y;

                                let js = build_js_drag(from_x, from_y, to_x, to_y);
                                let _ = page.evaluate(js).await;
                                did_drag = true;
                            }
                        }
                    }
                }

                // -----------------------------------------------------------------
                // Wait a little after the drag (or after the JS fallback).
                // -----------------------------------------------------------------
                if did_drag {
                    wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                        core::time::Duration::from_millis(900),
                    ))
                    .into();
                    wait_for.idle_network =
                        crate::features::chrome_common::WaitForIdleNetwork::new(
                            core::time::Duration::from_secs(6).into(),
                        )
                        .into();
                    wait_for.page_navigations = true;
                } else {
                    wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                        core::time::Duration::from_millis(650),
                    ))
                    .into();
                    wait_for.page_navigations = true;
                }

                let wait = Some(wait_for.clone());
                let _ = tokio::join!(
                    page_wait(page, &wait),
                    perform_smart_mouse_movement(page, viewport),
                );

                if let Ok(nc) = page.outer_html_bytes().await {
                    *b = nc;
                    if !looks_like_imperva_any(b.as_slice()) {
                        validated = true;
                        break;
                    }
                }

                continue;
            }

            // ---------------------------------------------------------
            // 3️⃣  Unknown interstitial – do a generic CF‑style wait and retry.
            // ---------------------------------------------------------
            wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                core::time::Duration::from_millis(900),
            ))
            .into();
            wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                core::time::Duration::from_secs(6).into(),
            )
            .into();
            wait_for.page_navigations = true;

            let wait = Some(wait_for.clone());
            let _ = tokio::join!(
                page_wait(page, &wait),
                perform_smart_mouse_movement(page, viewport),
            );

            if let Ok(nc) = page.outer_html_bytes().await {
                *b = nc;
                if !looks_like_imperva_any(b.as_slice()) {
                    validated = true;
                    break;
                }
            }
        }

        Ok::<(), chromiumoxide::error::CdpError>(())
    })
    .await;

    match page_result {
        Ok(_) => Ok(validated),
        _ => Err(chromiumoxide::error::CdpError::Timeout),
    }
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// Returns the `data:image/...;base64,…` string for the `<img>` whose
/// `src` attribute equals `src`.  The image is already loaded in the
/// page, so this does **not** trigger a network request – it draws the
/// image onto a temporary canvas and reads the data‑URL from that canvas.
async fn extract_image_dataurl(page: &chromiumoxide::Page, src: &str) -> Result<String, CdpError> {
    // JavaScript that receives the exact `src` value, finds the <img>,
    // draws it onto a canvas, and returns `canvas.toDataURL()`.
    let js = format!(
        r#"(function(){{
            const img = document.querySelector('img[src="{src}"]');
            if (!img) return null;
            const canvas = document.createElement('canvas');
            canvas.width = img.naturalWidth || img.width;
            canvas.height = img.naturalHeight || img.height;
            const ctx = canvas.getContext('2d');
            ctx.drawImage(img, 0, 0);
            return canvas.toDataURL();
        }})()"#
    );

    // `page.evaluate` returns an `EvaluationResult`.
    let eval = page.evaluate(js).await?;
    let dataurl = eval
        .value()
        .and_then(|v| v.as_str().map(|s| s.to_owned()))
        .ok_or_else(|| CdpError::msg("failed to extract tile data‑url"))?;

    Ok(dataurl)
}

/// High‑level wrapper – first tries the in‑page Gemini helper,
/// falls back to the external Gemini HTTP call when the helper is missing.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
pub async fn solve_enterprise_with_browser_gemini(
    page: &chromiumoxide::Page,
    challenge: &RcEnterpriseChallenge<'_>,
    timeout_ms: u64,
) -> Result<Vec<u8>, CdpError> {
    let mut tiles_json = Vec::with_capacity(challenge.tiles.len());

    for tile in &challenge.tiles {
        let dataurl = extract_image_dataurl(page, tile.img_src).await?;
        tiles_json.push(serde_json::json!({ "id": tile.id, "dataurl": dataurl }));
    }

    let target = challenge.target.unwrap_or("target object").to_string();

    match solve_with_inpage_helper(page, &tiles_json, &target, timeout_ms).await {
        Ok(ids) => return Ok(ids),
        Err(e) if !is_missing_helper_error(&e) => return Err(e),
        Err(_) => {} // helper missing → fall back
    }

    solve_with_external_gemini(challenge, timeout_ms)
        .await
        .map_err(|e| CdpError::msg(format!("external‑gemini failed: {e}")))
}

/// In‑page Gemini helper – receives tiles that already contain a
/// `dataurl` field (the image as a `data:image/...;base64,…` string).
#[cfg(all(feature = "chrome", feature = "real_browser"))]
async fn solve_with_inpage_helper(
    page: &chromiumoxide::Page,
    tiles_json: &[serde_json::Value],
    target: &str,
    timeout_ms: u64,
) -> Result<Vec<u8>, CdpError> {
    let script = format!(
        r#"
        async function solveRecaptchaEnterpriseWithGemini(tiles, target) {{
            const session = await LanguageModel.create({{
                expectedInputs: [
                    {{ type: "text", languages: ["en"] }},
                    {{ type: "image" }},
                ],
                expectedOutputs: [{{ type: "text", languages: ["en"] }}],
            }});
            const yesIds = [];
            for (const tile of tiles) {{
                const resp = await fetch(tile.dataurl);
                if (!resp.ok) continue;
                const blob = await resp.blob();
                const prompt = [{{
                    role: "user",
                    content: [
                        {{ type: "text", value: `Does this image contain a ${{
                            target
                        }}? Answer only with "yes" or "no".` }},
                        {{ type: "image", value: blob }},
                    ],
                }}];
                const answer = await session.prompt(prompt);
                const txt = (answer || "").toString().trim().toLowerCase();
                if (txt.includes("yes")) yesIds.push(tile.id);
            }}
            return yesIds;
        }}

        (async () => {{
            const result = await solveRecaptchaEnterpriseWithGemini(
                {tiles},
                {target}
            );
            return result;
        }})()
        "#,
        tiles = serde_json::to_string(tiles_json).unwrap(),
        target = serde_json::to_string(target).unwrap(),
    );

    // -----------------------------------------------------------------
    // Ask Chrome to evaluate the script (same timeout logic as before).
    // -----------------------------------------------------------------
    let params = EvaluateParams::builder()
        .expression(&script)
        .await_promise(true)
        .build()
        .unwrap();

    let eval_fut = page.evaluate(params);
    let eval_res = tokio::time::timeout(Duration::from_millis(timeout_ms + 5_000), eval_fut)
        .await
        .map_err(|_| CdpError::Timeout)?;

    match eval_res {
        Ok(eval) => match eval.value() {
            Some(serde_json::Value::Array(arr)) => {
                let ids = arr
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u8))
                    .collect();
                Ok(ids)
            }
            _ => Ok(vec![]),
        },
        Err(e) => Err(e),
    }
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// Is the language model missing.
fn is_missing_helper_error(err: &CdpError) -> bool {
    let txt = format!("{err}");
    txt.contains("LanguageModel is not defined")
        || txt.contains("ReferenceError")
        || txt.contains("Uncaught ReferenceError")
        || txt.contains("cannot read property 'create' of undefined")
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// Extract gemini fallback.
async fn solve_with_external_gemini(
    challenge: &RcEnterpriseChallenge<'_>,
    timeout_ms: u64,
) -> Result<Vec<u8>, RequestError> {
    if let Ok(api_key) = std::env::var("GEMINI_API_KEY") {
        if let Ok(_sem) = crate::utils::GEMINI_SEM
            .acquire_many(challenge.tiles.len().try_into().unwrap_or(1))
            .await
        {
            let endpoint = format!("{}?key={}", *GEMINI_VISION_ENDPOINT, api_key);

            let target = challenge.target.unwrap_or("target object").to_string();

            let mut yes_ids = Vec::new();

            for tile in &challenge.tiles {
                // -------------------------------------------------------------
                // a) Download the image bytes.
                // -------------------------------------------------------------
                let img_bytes = match GEMINI_CLIENT.get(tile.img_src).send().await {
                    Ok(resp) if resp.status().is_success() => resp.bytes().await?,
                    _ => continue, // if we cannot fetch the image we just skip it
                };

                // -------------------------------------------------------------
                // b) Build the Gemini request body.
                // -------------------------------------------------------------
                // Gemini expects a JSON object with a `contents` array.
                // Each element contains a `parts` array.  We send one text part and
                // one image part (base64‑encoded).
                let request_body = serde_json::json!({
                    "contents": [{
                        "role": "user",
                        "parts": [
                            {
                                "text": format!("Does this image contain a {}? Answer only with \"yes\" or \"no\".", target)
                            },
                            {
                                "inlineData": {
                                    "mimeType": "image/jpeg",   // recaptcha images are JPEGs
                                    "data": BASE64_STANDARD.encode(&img_bytes)
                                }
                            }
                        ]
                    }],
                    // The model may be asked to stop after it emits the answer.
                    "generationConfig": {
                        "maxOutputTokens": 5,
                        "temperature": 0.0
                    }
                });

                // -------------------------------------------------------------
                // c) Send the request (with a per‑tile timeout that is a fraction of
                //    the total timeout we were given).
                // -------------------------------------------------------------
                let per_tile_timeout =
                    Duration::from_millis(timeout_ms / (challenge.tiles.len() as u64 + 1));
                let resp = tokio::time::timeout(
                    per_tile_timeout,
                    GEMINI_CLIENT.post(&endpoint).json(&request_body).send(),
                )
                .await;

                let resp = match resp {
                    Ok(Ok(r)) => r,
                    // Either the HTTP request timed out or returned an error – skip.
                    _ => continue,
                };

                // -------------------------------------------------------------
                // d) Parse the Gemini answer.
                // -------------------------------------------------------------
                let resp_json: serde_json::Value = resp.json().await?;
                // The answer text lives in `candidates[0].content.parts[0].text`.
                let answer_text = resp_json
                    .get("candidates")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("content"))
                    .and_then(|c| c.get("parts"))
                    .and_then(|p| p.get(0))
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();

                if answer_text.contains("yes") {
                    yes_ids.push(tile.id);
                }
            }

            Ok(yes_ids)
        } else {
            Ok(Vec::new())
        }
    } else {
        Ok(Vec::new())
    }
}

/// Warm‑up the in‑page Gemini `LanguageModel` for the given Chrome page.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
pub async fn warm_gemini_model(page: &Page) -> Result<(), CdpError> {
    let eval_params = EvaluateParams::builder()
        .expression(r#"(async()=>{try{const s=await LanguageModel.create({expectedInputs:[{type:"text",languages:["en"]}],expectedOutputs:[{type:"text",languages:["en"]}]});await s.prompt([{role:"user",content:[{type:"text",value:"ping"}]}])}catch(_){}})()"#)
        .await_promise(true)
        .build()
        .expect("valid evaluate params");

    tokio::time::timeout(Duration::from_secs(60), page.evaluate(eval_params))
        .await
        .map_err(|_| CdpError::Timeout)??;

    Ok(())
}

/// Handle reCAPTCHA checkbox (anchor iframe) via chrome.
/// This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
#[inline(always)]
pub async fn recaptcha_handle(
    b: &mut Vec<u8>,
    page: &chromiumoxide::Page,
    viewport: &Option<crate::configuration::Viewport>,
) -> Result<bool, CdpError> {
    if !detect_recaptcha(b.as_slice()) {
        return Ok(false);
    }

    let mut validated = false;

    let overall = tokio::time::timeout(Duration::from_secs(30), async {
        // Keep the mouse moving a little – helps not being flagged as a bot.
        let _ = tokio::join!(
            page.disable_network_cache(true),
            perform_smart_mouse_movement(page, viewport)
        );

        for _ in 0..10 {
            // ---------------------------------------------------------
            // a) Refresh HTML into the caller’s buffer.
            // ---------------------------------------------------------
            if let Ok(cur) = page.outer_html_bytes().await {
                *b = cur;
            }

            // ---------------------------------------------------------
            // b) If Recaptcha vanished → success.
            // ---------------------------------------------------------
            if !detect_recaptcha(b.as_slice()) {
                validated = true;
                break;
            }

            // ---------------------------------------------------------
            // c) **Enterprise** handling – now solved with the built‑in Gemini.
            // ---------------------------------------------------------
            if extract_rc_enterprise_challenge(b.as_slice()).is_some() {
                // 1️⃣  Ensure the anchor iframe exists (first click).
                let anchor_present = page
                    .find_elements_pierced(r#"iframe[src*="/recaptcha/api2/anchor"]"#)
                    .await
                    .map(|els| !els.is_empty())
                    .unwrap_or(false);

                if !anchor_present {
                    // Wait for it to appear – same CF‑style wait.
                    let mut wait_for = CF_WAIT_FOR.clone();
                    wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                        Duration::from_millis(900),
                    ))
                    .into();
                    wait_for.idle_network =
                        crate::features::chrome_common::WaitForIdleNetwork::new(
                            Duration::from_secs(6).into(),
                        )
                        .into();
                    wait_for.page_navigations = true;
                    let wait = Some(wait_for.clone());
                    let _ = tokio::join!(
                        page_wait(page, &wait),
                        perform_smart_mouse_movement(page, viewport),
                    );
                    continue; // retry outer loop
                }

                // 2️⃣  Click the classic checkbox (same logic as before).
                async fn click_anchor(page: &chromiumoxide::Page) -> bool {
                    if let Ok(els) = page.find_elements_pierced(r#"#recaptcha-anchor"#).await {
                        if let Some(el) = els.into_iter().next() {
                            return match el.clickable_point().await {
                                Ok(p) => page.click(p).await.is_ok() || el.click().await.is_ok(),
                                Err(_) => el.click().await.is_ok(),
                            };
                        }
                    }
                    if let Ok(els) = page
                        .find_elements_pierced(r#".recaptcha-checkbox-checkmark"#)
                        .await
                    {
                        if let Some(el) = els.into_iter().next() {
                            return match el.clickable_point().await {
                                Ok(p) => page.click(p).await.is_ok() || el.click().await.is_ok(),
                                Err(_) => el.click().await.is_ok(),
                            };
                        }
                    }
                    false
                }

                let clicked = click_anchor(page).await;

                // 3️⃣  Wait a bit for the grid iframe to load.
                let mut wait_for = CF_WAIT_FOR.clone();
                wait_for.delay =
                    crate::features::chrome_common::WaitForDelay::new(Some(if clicked {
                        Duration::from_millis(1_100)
                    } else {
                        Duration::from_millis(700)
                    }))
                    .into();
                wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                    Duration::from_secs(7).into(),
                )
                .into();
                wait_for.page_navigations = true;
                let wait = Some(wait_for.clone());
                let _ = tokio::join!(
                    page_wait(page, &wait),
                    perform_smart_mouse_movement(page, viewport),
                );

                // ---------------------------------------------------------
                // d) Grab the grid HTML again – we need the *latest* tile URLs.
                // ---------------------------------------------------------
                let grid_html = match page.outer_html_bytes().await {
                    Ok(h) => h,
                    Err(_) => continue,
                };
                *b = grid_html.clone();

                // If the grid disappeared after the click, we’re done.
                if !detect_recaptcha(b.as_slice()) {
                    validated = true;
                    break;
                }

                // Extract the challenge *again* (now we are sure the grid is present).
                let challenge = match extract_rc_enterprise_challenge(&grid_html) {
                    Some(c) => c,
                    None => continue,
                };

                // ---------------------------------------------------------
                // e) **Solve with the built‑in Gemini** (the function above).
                // ---------------------------------------------------------
                let yes_ids = solve_enterprise_with_browser_gemini(page, &challenge, 20_000)
                    .await
                    .map_err(|e| {
                        CdpError::ChromeMessage(format!("gemini in‑page failed: {}", e))
                    })?;

                // ---------------------------------------------------------
                // f) Click every tile that received a “yes”.
                // ---------------------------------------------------------
                for id in yes_ids {
                    if let Some(tile) = challenge.tiles.iter().find(|t| t.id == id) {
                        // Build a selector that matches the exact `<img src="…">`.
                        let selector = format!(r#"img[src="{}"]"#, tile.img_src);
                        if let Ok(els) = page.find_elements_pierced(&selector).await {
                            if let Some(el) = els.into_iter().next() {
                                let _ = el.click().await; // ignore possible errors
                            }
                        }
                    }
                }

                // ---------------------------------------------------------
                // g) Click the Verify button if it exists.
                // ---------------------------------------------------------
                if challenge.has_verify_button {
                    if let Ok(btns) = page
                        .find_elements_pierced(
                            r#"button[id*="recaptcha-verify-button"], button:contains("Verify")"#,
                        )
                        .await
                    {
                        if let Some(btn) = btns.into_iter().next() {
                            let _ = btn.click().await;
                        }
                    }
                }

                // ---------------------------------------------------------
                // h) Final wait for navigation / network idle.
                // ---------------------------------------------------------
                let mut wait_for = CF_WAIT_FOR.clone();
                wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                    Duration::from_millis(1_500),
                ))
                .into();
                wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                    Duration::from_secs(8).into(),
                )
                .into();
                wait_for.page_navigations = true;
                let wait = Some(wait_for.clone());
                let _ = tokio::join!(
                    page_wait(page, &wait),
                    perform_smart_mouse_movement(page, viewport),
                );

                // ---------------------------------------------------------
                // i) Refresh HTML one last time – if the whole Recaptcha is gone we’re finished.
                // ---------------------------------------------------------
                if let Ok(new_html) = page.outer_html_bytes().await {
                    *b = new_html;
                    if !detect_recaptcha(b.as_slice()) {
                        validated = true;
                        break;
                    }
                }

                // If we are still here the grid is still present – loop again (maybe a slider appears).
                continue;
            }

            let anchor_iframe_present = page
                .find_elements_pierced(r#"iframe[src*="/recaptcha/api2/anchor"]"#)
                .await
                .map(|els| !els.is_empty())
                .unwrap_or(false);

            if !anchor_iframe_present {
                let mut wait_for = CF_WAIT_FOR.clone();
                wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                    Duration::from_millis(900),
                ))
                .into();
                wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                    Duration::from_secs(6).into(),
                )
                .into();
                wait_for.page_navigations = true;
                let wait = Some(wait_for.clone());
                let _ = tokio::join!(
                    page_wait(page, &wait),
                    perform_smart_mouse_movement(page, viewport),
                );
                continue;
            }

            // Click the classic checkbox (same logic you already had)
            let mut clicked = false;
            if let Ok(els) = page.find_elements_pierced(r#"#recaptcha-anchor"#).await {
                if let Some(el) = els.into_iter().next() {
                    clicked = match el.clickable_point().await {
                        Ok(p) => page.click(p).await.is_ok() || el.click().await.is_ok(),
                        Err(_) => el.click().await.is_ok(),
                    };
                }
            }
            if !clicked {
                if let Ok(els) = page
                    .find_elements_pierced(r#".recaptcha-checkbox-checkmark"#)
                    .await
                {
                    if let Some(el) = els.into_iter().next() {
                        clicked = match el.clickable_point().await {
                            Ok(p) => page.click(p).await.is_ok() || el.click().await.is_ok(),
                            Err(_) => el.click().await.is_ok(),
                        };
                    }
                }
            }

            let mut wait_for = CF_WAIT_FOR.clone();
            wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(if clicked {
                Duration::from_millis(1_100)
            } else {
                Duration::from_millis(700)
            }))
            .into();
            wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                Duration::from_secs(7).into(),
            )
            .into();
            wait_for.page_navigations = true;
            let wait = Some(wait_for.clone());
            let _ = tokio::join!(
                page_wait(page, &wait),
                perform_smart_mouse_movement(page, viewport),
            );

            if let Ok(new_html) = page.outer_html_bytes().await {
                *b = new_html;
                if !detect_recaptcha(b.as_slice()) {
                    validated = true;
                    break;
                }
            }
        }

        Ok::<(), CdpError>(())
    })
    .await;

    match overall {
        Ok(_) => Ok(validated),
        Err(_) => Err(CdpError::Timeout),
    }
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// Remove solve lemin external.
pub async fn solve_lemin_with_external_gemini(image_dataurl: &str, timeout_ms: u64) -> (f64, f64) {
    let api_key = match std::env::var("GEMINI_API_KEY") {
        Ok(k) => k,
        Err(_) => return (0.0, 0.0),
    };

    /* ----------------------------------------------------------------- *
     * 2️⃣  Acquire the semaphore that throttles concurrent Gemini calls.
     * ----------------------------------------------------------------- */
    if crate::utils::GEMINI_SEM.acquire().await.is_err() {
        return (0.0, 0.0);
    }

    /* ----------------------------------------------------------------- *
     * 3️⃣  Decode the `data:` URL into raw PNG bytes.
     * ----------------------------------------------------------------- */
    let b64_part = match image_dataurl.split_once(',').map(|x| x.1) {
        Some(p) => p.trim(),
        None => return (0.0, 0.0),
    };

    let img_bytes = match BASE64_STANDARD.decode(b64_part) {
        Ok(b) => b,
        Err(_) => return (0.0, 0.0),
    };

    /* ----------------------------------------------------------------- *
     * 4️⃣  Build the Gemini request payload – we ask for a JSON array `[x,y]`.
     * ----------------------------------------------------------------- */
    let request_body = serde_json::json!({
        "contents": [{
            "role": "user",
            "parts": [
                {
                    "text": "Give me the centre (x and y coordinates) of the missing puzzle piece in this image. Return a JSON array like [x, y] with numbers only."
                },
                {
                    "inlineData": {
                        "mimeType": "image/png",
                        "data": BASE64_STANDARD.encode(&img_bytes)
                    }
                }
            ]
        }],
        "generationConfig": {
            "maxOutputTokens": 16,
            "temperature": 0.0
        }
    });

    /* ----------------------------------------------------------------- *
     * 5️⃣  Perform the HTTP request, respecting a per‑tile timeout.
     * ----------------------------------------------------------------- */
    let per_tile = Duration::from_millis(timeout_ms / 2);
    let resp = match tokio::time::timeout(
        per_tile,
        GEMINI_CLIENT
            .post(&*GEMINI_VISION_ENDPOINT)
            .header("x-goog-api-key", api_key)
            .json(&request_body)
            .send(),
    )
    .await
    {
        Ok(Ok(r)) => r,
        _ => return (0.0, 0.0), // timeout or transport error
    };

    /* ----------------------------------------------------------------- *
     * 6️⃣  Verify we received a 200 OK.
     * ----------------------------------------------------------------- */
    if !resp.status().is_success() {
        return (0.0, 0.0);
    }

    /* ----------------------------------------------------------------- *
     * 7️⃣  Pull the textual answer out of Gemini’s JSON envelope.
     * ----------------------------------------------------------------- */
    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return (0.0, 0.0),
    };

    let txt = match json
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.get(0))
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())
    {
        Some(t) => t.trim(),
        None => return (0.0, 0.0),
    };

    /* ----------------------------------------------------------------- *
     * 8️⃣  Parse the `[x, y]` JSON array we asked Gemini for.
     * ----------------------------------------------------------------- */
    let coords: Vec<f64> = match serde_json::from_str(txt) {
        Ok(v) => v,
        Err(_) => return (0.0, 0.0),
    };

    if coords.len() == 2 {
        (coords[0], coords[1])
    } else {
        (0.0, 0.0)
    }
}

/// Lemin in page helper.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
async fn solve_lemin_with_inpage_helper(
    page: &Page,
    image_dataurl: &str,
    timeout_ms: u64,
) -> Result<(f64, f64), CdpError> {
    let script = format!(
        r#"(async () => {{
            try {{
                const session = await LanguageModel.create({{
                    expectedInputs: [
                        {{ type: "text", languages: ["en"] }},
                        {{ type: "image" }},
                    ],
                    expectedOutputs: [{{ type: "text", languages: ["en"] }}],
                }});
                const imgResp = await fetch("{image_dataurl}");
                if (!imgResp.ok) return null;
                const blob = await imgResp.blob();
                const prompt = [{{
                    role: "user",
                    content: [
                        {{ type: "text", value: "Give me the centre (x and y coordinates) of the missing puzzle piece in this image. Return a JSON array like [x, y] with numbers only." }},
                        {{ type: "image", value: blob }},
                    ],
                }}];
                const answer = await session.prompt(prompt);
                const txt = (answer || "").toString().trim();
                try {{ return JSON.parse(txt); }}
                catch {{ return null; }}
            }} catch (e) {{ throw e; }}
        }})()"#
    );

    let eval_fut = page.evaluate(
        EvaluateParams::builder()
            .expression(&script)
            .await_promise(true)
            .build()
            .unwrap(),
    );

    let eval_res = tokio::time::timeout(Duration::from_millis(timeout_ms + 5_000), eval_fut)
        .await
        .map_err(|_| CdpError::Timeout)?; // outer timeout → CdpError::Timeout

    match eval_res {
        Ok(eval) => match eval.value() {
            Some(serde_json::Value::Array(arr)) if arr.len() == 2 => {
                let x = arr[0]
                    .as_f64()
                    .ok_or_else(|| CdpError::msg("Gemini did not return a numeric x"))?;
                let y = arr[1]
                    .as_f64()
                    .ok_or_else(|| CdpError::msg("Gemini did not return a numeric y"))?;
                Ok((x, y))
            }
            _ => Err(CdpError::msg("Gemini did not return a valid [x, y] array")),
        },
        Err(e) => Err(e), // propagate Chrome errors (including missing helper)
    }
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// Lemin solve handler.
pub async fn lemin_handle(
    b: &mut Vec<u8>,
    page: &Page,
    viewport: &Option<crate::configuration::Viewport>,
) -> Result<bool, CdpError> {
    // -----------------------------------------------------------------
    // Fast‑gate – bail out early if the page does not contain a Lemin widget.
    // -----------------------------------------------------------------
    if !detect_lemin(b.as_slice()) {
        return Ok(false);
    }

    let mut progressed = false;

    // -----------------------------------------------------------------
    // Whole routine lives inside a 30 s timeout (same pattern as the rest).
    // -----------------------------------------------------------------
    let page_result = tokio::time::timeout(Duration::from_secs(30), async {
        // Disable cache + a little “human” mouse movement.
        let _ = tokio::join!(
            page.disable_network_cache(true),
            perform_smart_mouse_movement(page, viewport)
        );

        for _ in 0..10 {
            // ---------------------------------------------------------
            // a) Refresh the HTML source.
            // ---------------------------------------------------------
            if let Ok(cur) = page.outer_html_bytes().await {
                *b = cur;
            }

            // ---------------------------------------------------------
            // b) If Lemin vanished → success.
            // ---------------------------------------------------------
            if !detect_lemin(b.as_slice()) {
                progressed = true;
                break;
            }

            // ---------------------------------------------------------
            // c) Click the hidden checkbox that activates the puzzle.
            // ---------------------------------------------------------
            if let Ok(checkboxes) = page
                .find_elements_pierced(
                    r#"div[id*="lemin-cropped-captcha"] input[type="checkbox"]"#,
                )
                .await
            {
                if let Some(cb) = checkboxes.into_iter().next() {
                    let clicked = match cb.clickable_point().await {
                        Ok(p) => page.click(p).await.is_ok() || cb.click().await.is_ok(),
                        Err(_) => cb.click().await.is_ok(),
                    };
                    if clicked {
                        let mut wait_for = CF_WAIT_FOR.clone();
                        wait_for.delay =
                            crate::features::chrome_common::WaitForDelay::new(Some(
                                Duration::from_millis(900),
                            ))
                            .into();
                        wait_for.idle_network =
                            crate::features::chrome_common::WaitForIdleNetwork::new(
                                Duration::from_secs(6).into(),
                            )
                            .into();
                        wait_for.page_navigations = true;
                        let wait = Some(wait_for.clone());
                        let _ = tokio::join!(
                            page_wait(page, &wait),
                            perform_smart_mouse_movement(page, viewport),
                        );
                    }
                }
            }

            // ---------------------------------------------------------
            // d) Locate the **full background image** and turn it into a data‑URL.
            // ---------------------------------------------------------
            let img_el = match page
                .find_elements_pierced(
                    r#"div[id*="lemin-captcha-popup"] img[src][width][height]"#,
                )
                .await
            {
                Ok(mut els) => els.pop(),
                Err(_) => None,
            };

            let dataurl = if let Some(img) = &img_el {
                // Use a temporary canvas to read the image as a data‑URL.
                let call = CallFunctionOnParams::builder()
                    .object_id(img.remote_object_id.clone())
                    .function_declaration(
                        "(function(){ const canvas = document.createElement('canvas'); \
                           canvas.width = this.naturalWidth || this.width; \
                           canvas.height = this.naturalHeight || this.height; \
                           const ctx = canvas.getContext('2d'); \
                           ctx.drawImage(this,0,0); \
                           return canvas.toDataURL(); })",
                    )
                    .await_promise(true)
                    .build()
                    .unwrap();

                let eval = page.evaluate_function(call).await?;
                eval.value()
                    .and_then(|v| v.as_str().map(|s| s.to_owned()))
                    .ok_or_else(|| CdpError::msg("failed to get data‑url from Lemin image"))?
            } else {
                return Err(CdpError::msg(
                    "Lemin puzzle image not found – cannot continue",
                ));
            };

            // ---------------------------------------------------------
            // e) Ask Gemini for the missing piece centre (x, y) – first try the
            //    in‑page helper, then fall back to the remote call.
            // ---------------------------------------------------------
            let (target_x, target_y) = match solve_lemin_with_inpage_helper(page, &dataurl, 20_000).await {
                Ok(p) => p,
                Err(e) if is_missing_helper_error(&e) => {
                    // -----------------------------------------------------------------
                    // Remote fallback – this is the same behaviour you already have
                    // for Recaptcha‑Enterprise.
                    // -----------------------------------------------------------------
                    solve_lemin_with_external_gemini(&dataurl, 20_000)
                        .await
                }
                Err(e) => return Err(e), // any other Chrome error bubbles up
            };

            // ---------------------------------------------------------
            // f) Locate the **draggable piece**.
            // ---------------------------------------------------------
            let piece_el = match page
                .find_elements_pierced(
                    r#"div[style*="touch-action: none"][style*="cursor: move"][style*="position: absolute"]"#,
                )
                .await
            {
                Ok(mut els) => els.pop(),
                Err(_) => None,
            };

            let piece_bb = if let Some(el) = piece_el {
                el.bounding_box().await?
            } else {
                return Err(CdpError::msg(
                    "Lemin draggable piece not found – cannot solve",
                ));
            };

            // ---------------------------------------------------------
            // g) Transform the coordinates returned by Gemini (relative to the
            //    full image) into absolute page coordinates.
            // ---------------------------------------------------------
            let img_bb = if let Some(img) = &img_el {
                img.bounding_box().await?
            } else {
                return Err(CdpError::msg(
                    "Lemin full image missing when calculating drag target",
                ));
            };

            // The image might be scaled, so compute a scale factor.
            let scale_x = img_bb.width / img_bb.width.max(1.0);
            let scale_y = img_bb.height / img_bb.height.max(1.0);
            let page_target_x = img_bb.x + target_x * scale_x;
            let page_target_y = img_bb.y + target_y * scale_y;

            // ---------------------------------------------------------
            // h) Drag the piece to the target.
            // ---------------------------------------------------------
            let from = Point {
                x: piece_bb.x + piece_bb.width * 0.5,
                y: piece_bb.y + piece_bb.height * 0.5,
            };
            let to = Point {
                x: page_target_x,
                y: page_target_y,
            };
            let _ = page.click_and_drag(from, to).await;

            // ---------------------------------------------------------
            // i) Click the **Verify** button (if present).
            // ---------------------------------------------------------
            if let Ok(btns) = page
                .find_elements_pierced(r#"button.verify-button, button[id*="verify-button"]"#)
                .await
            {
                if let Some(btn) = btns.into_iter().next() {
                    let _ = btn.click().await;
                }
            }

            // ---------------------------------------------------------
            // j) Wait a little, then check whether the widget disappeared.
            // ---------------------------------------------------------
            let mut wf = CF_WAIT_FOR.clone();
            wf.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                Duration::from_millis(1_100),
            ))
            .into();
            wf.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                Duration::from_secs(7).into(),
            )
            .into();
            wf.page_navigations = true;
            let wait = Some(wf.clone());
            let _ = tokio::join!(
                page_wait(page, &wait),
                perform_smart_mouse_movement(page, viewport),
            );

            // ---------------------------------------------------------
            // k) Final check – if the Lemin widget vanished we are done.
            // ---------------------------------------------------------
            if let Ok(nc2) = page.outer_html_bytes().await {
                *b = nc2;
                if !detect_lemin(b.as_slice()) {
                    progressed = true;
                    break;
                }
            }

            // If we get here the puzzle was not solved – the outer loop will retry.
        }

        Ok::<(), CdpError>(())
    })
    .await;

    match page_result {
        Ok(_) => Ok(progressed),
        Err(_) => Err(CdpError::Timeout),
    }
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
#[derive(Debug, Clone)]
/// The RC tile reference.
pub struct RcTileRef<'a> {
    /// The id.
    pub id: u8,
    /// The img src.
    pub img_src: &'a str,
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// Enterprise challenge.
#[derive(Debug, Default, Clone)]
pub struct RcEnterpriseChallenge<'a> {
    /// e.g. "bridges" (from `<strong>bridges</strong>`)
    pub target: Option<&'a str>,
    /// full instruction line if you want it
    pub instruction_text: Option<&'a str>,
    /// The tile space.
    pub tiles: Vec<RcTileRef<'a>>,
    /// Has the verification button.
    pub has_verify_button: bool,
}

/// Byte‑wise equality (fast, zero‑allocation).  
/// Returns `true` iff `a` and `b` have the same length **and** identical bytes.
#[inline(always)]
#[cfg(all(feature = "chrome", feature = "real_browser"))]
fn memeq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y)
}

/// Search for `needle` in `haystack` starting at `start`.  
/// Returns the absolute index of the first match or `None` if not found.
#[inline(always)]
#[cfg(all(feature = "chrome", feature = "real_browser"))]
fn find(h: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    let nl = needle.len();
    if nl == 0 || start >= h.len() || nl > h.len() - start {
        return None;
    }
    h[start..]
        .windows(nl)
        .position(|w| memeq(w, needle))
        .map(|p| start + p)
}

/// Find the next double‑quote (`"`) after `start`.  
/// Returns its absolute index or `None` if missing.
#[inline(always)]
#[cfg(all(feature = "chrome", feature = "real_browser"))]
fn find_quote_end(h: &[u8], start: usize) -> Option<usize> {
    h.get(start..)?
        .iter()
        .position(|&c| c == b'"')
        .map(|p| start + p)
}

/// Is `b` an ASCII digit (`0`‑`9`)?
#[inline(always)]
#[cfg(all(feature = "chrome", feature = "real_browser"))]
fn is_digit(b: u8) -> bool {
    b.is_ascii_digit()
}

/// Convert a single ASCII digit to `u8`. Returns `None` for non‑digits.
#[inline(always)]
#[cfg(all(feature = "chrome", feature = "real_browser"))]
fn parse_u8_1digit(b: u8) -> Option<u8> {
    if is_digit(b) {
        Some(b - b'0')
    } else {
        None
    }
}

/// Extracts recaptcha enterprise image-grid metadata from the iframe inner HTML.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
#[inline(always)]
pub fn extract_rc_enterprise_challenge<'a>(html: &'a [u8]) -> Option<RcEnterpriseChallenge<'a>> {
    // -----------------------------------------------------------------
    // Quick gate – all four guard patterns must be present.
    // -----------------------------------------------------------------
    // `RC_ENTERPRISE_GUARD_AC` contains the four patterns in the order
    // they appear in `RC_ENTERPRISE_GUARD_PATTERNS`.  We check each one
    // individually because we need **all** of them.
    let mut guard_hits = [false; 4];
    for m in RC_ENTERPRISE_GUARD_AC.find_iter(html) {
        guard_hits[m.pattern()] = true;
    }
    if !guard_hits.iter().all(|&b| b) {
        return None;
    }

    // -----------------------------------------------------------------
    // Does the page have a “Verify” button?
    // -----------------------------------------------------------------
    let has_verify_button = RC_VERIFY_BUTTON_AC.is_match(html);

    let mut out = RcEnterpriseChallenge {
        target: None,
        instruction_text: None,
        tiles: Vec::with_capacity(12),
        has_verify_button,
    };

    // -----------------------------------------------------------------
    // 1️⃣  Extract the *target* word (the word that appears inside the
    //      <strong …> … </strong> that is near the description).
    // -----------------------------------------------------------------
    const DESC_PAT: &[u8] = b"rc-imageselect-desc";
    const STRONG_OPEN: &[u8] = b"<strong";
    const GT: &[u8] = b">";
    const STRONG_CLOSE: &[u8] = b"</strong>";

    if let Some(desc_pos) = find(html, DESC_PAT, 0) {
        // Look forward a bounded window for the <strong> element.
        let win_end = (desc_pos + 900).min(html.len());

        if let Some(strong_pos) = find(html, STRONG_OPEN, desc_pos) {
            if strong_pos < win_end {
                if let Some(gt_pos) = find(html, GT, strong_pos) {
                    let txt_start = gt_pos + 1;
                    if let Some(close_pos) = find(html, STRONG_CLOSE, txt_start) {
                        if close_pos <= win_end {
                            if let Ok(word) = core::str::from_utf8(&html[txt_start..close_pos]) {
                                let word = word.trim();
                                if !word.is_empty() {
                                    out.target = Some(word);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Optional – full description text (everything between the first ‘>’
        // after the descriptor and the next ‘<’).
        if let Some(tag_end) = find(html, b">", desc_pos) {
            let t0 = tag_end + 1;
            if let Some(t1) = find(html, b"<", t0) {
                if let Ok(txt) = core::str::from_utf8(&html[t0..t1]) {
                    let txt = txt.trim();
                    if !txt.is_empty() {
                        out.instruction_text = Some(txt);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // 2️⃣  Extract every tile (id + image URL).
    // -----------------------------------------------------------------
    const ID_PAT: &[u8] = b"id=\"";
    const SRC_PAT: &[u8] = b"src=\"";
    const PAYLOAD_PREFIX: &[u8] = b"https://www.google.com/recaptcha/enterprise/payload";

    // `RC_TILE_CLASS_AC` yields the start offset of every occurrence of
    // `rc-imageselect-tile`.  We iterate over those offsets instead of the
    // previous while‑loop that scanned the whole buffer.
    for m in RC_TILE_CLASS_AC.find_iter(html) {
        let tile_pos = m.start();

        // Back‑scan (max 240 bytes) for the id attribute that belongs to this tile.
        let back = tile_pos.saturating_sub(240);
        let id_pos = match find(html, ID_PAT, back) {
            Some(p) if p < tile_pos => p,
            _ => continue,
        };
        // The id is a single digit (0‑9) in the official widget.
        let id = match html
            .get(id_pos + ID_PAT.len())
            .copied()
            .and_then(parse_u8_1digit)
        {
            Some(v) => v,
            None => continue,
        };

        // Find the image src *after* the tile marker.
        let src_pos = match find(html, SRC_PAT, tile_pos) {
            Some(p) => p,
            None => continue,
        };
        let url_start = src_pos + SRC_PAT.len();

        // Ensure the URL really points to the Enterprise payload endpoint.
        if html.get(url_start..url_start + PAYLOAD_PREFIX.len()) != Some(PAYLOAD_PREFIX) {
            continue;
        }

        // The URL ends at the next double‑quote.
        let url_end = match find_quote_end(html, url_start) {
            Some(e) => e,
            None => continue,
        };
        let url = match core::str::from_utf8(&html[url_start..url_end]) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // De‑duplicate tiles that may re‑appear after a re‑render.
        if !out.tiles.iter().any(|t| t.id == id) {
            out.tiles.push(RcTileRef { id, img_src: url });
        }
    }

    if out.tiles.is_empty() {
        None
    } else {
        Some(out)
    }
}
#[cfg(feature = "gemini")]
mod gemini {
    use super::*;
    // ----  no `anyhow` import any more  ----
    use serde::{Deserialize, Serialize};

    #[derive(Serialize)]
    struct Payload<'a> {
        /// Base‑64 data URL of the canvas (`data:image/png;base64,…`).
        image: &'a str,
        /// Prompt that makes Gemini return the **horizontal pixel offset** of the
        /// missing piece.
        prompt: &'static str,
    }

    #[derive(Deserialize)]
    struct GeminiResponse {
        /// X‑offset of the gap (relative to the left edge of the image).
        x: f64,
    }

    /// Calls Gemini‑Pro‑Vision and returns the x‑coordinate of the gap.
    ///
    /// The function now returns a plain `Result<f64, Box<dyn std::error::Error>>`,
    /// which works with the `?` operator for every error type that `reqwest`
    /// (and `serde_json`) may produce.
    pub async fn solve_with_gemini(
        api_key: &str,
        image_dataurl: &str,
    ) -> Result<f64, Box<dyn std::error::Error>> {
        // Prompt that works best for GeeTest sliders.
        const PROMPT: &str = r#"
You are shown a screenshot of a GeeTest sliding‑puzzle captcha.
The image contains a background with a single missing puzzle piece cut‑out.
Return **only** the horizontal pixel offset (integer or float) of the left edge of the missing piece
measured from the left border of the image.
Do NOT return any extra text, JSON keys, or explanations.
"#;

        let payload = Payload {
            image: image_dataurl,
            prompt: PROMPT,
        };

        let url = format!(
            "{}:generateContent?key={}",
            *GEMINI_VISION_ENDPOINT, api_key
        );

        // All intermediate errors (`reqwest::Error`, `serde_json::Error`, …)
        // are automatically converted into `Box<dyn Error>` via the `From`
        // implementations that the standard library provides.
        let resp = GEMINI_CLIENT
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?
            .json::<GeminiResponse>()
            .await?;

        Ok(resp.x)
    }
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// In page geetest helper.
pub async fn solve_geetest_with_inpage_helper(
    page: &Page,
    canvas_dataurl: &str,
    timeout_ms: u64,
) -> Result<f64, CdpError> {
    // -----------------------------------------------------------------
    // 1️⃣  Encode the data‑url as a JSON string so that it can be safely
    //     interpolated into the JS source.
    // -----------------------------------------------------------------
    let js_literal = serde_json::to_string(canvas_dataurl)
        .map_err(|e| CdpError::msg(format!("JSON encode error: {e}")))?;

    // -----------------------------------------------------------------
    // 2️⃣  The in‑page helper script.
    // -----------------------------------------------------------------
    //    • Creates a `LanguageModel` (the same model Chrome exposes to
    //      extensions).
    //    • Downloads the image from the data‑url, sends it together with a
    //      short prompt that asks for *only* the horizontal offset.
    //    • Returns that offset as a plain number (or `null` on any error).
    // -----------------------------------------------------------------
    let script = format!(
        r#"(async () => {{
            try {{
                const session = await LanguageModel.create({{
                    expectedInputs: [
                        {{ type: "image" }},
                        {{ type: "text", languages: ["en"] }},
                    ],
                    expectedOutputs: [{{ type: "text", languages: ["en"] }}],
                }});
                const imgResp = await fetch({js_literal});
                if (!imgResp.ok) return null;
                const blob = await imgResp.blob();

                const prompt = [{{
                    role: "user",
                    content: [
                        {{ type: "image", value: blob }},
                        {{ type: "text", value: "Return only the horizontal pixel offset (as a number) of the missing puzzle piece gap in this image." }},
                    ],
                }}];

                const answer = await session.prompt(prompt);
                const txt = (answer ?? "").toString().trim();
                const num = parseFloat(txt);
                return isNaN(num) ? null : num;
            }} catch (e) {{
                throw e;
            }}
        }})()"#
    );

    let eval_fut = page.evaluate(
        EvaluateParams::builder()
            .expression(&script)
            .await_promise(true)
            .build()
            .unwrap(),
    );

    let eval_outcome = tokio::time::timeout(Duration::from_millis(timeout_ms + 5_000), eval_fut)
        .await
        .map_err(|_| CdpError::Timeout)?; // outer timeout → CdpError::Timeout

    // -----------------------------------------------------------------
    // 4️⃣  Distinguish three cases:
    //     a) The script succeeded (`Ok(EvaluationResult)`).
    //     b) The script threw → we get `Err(CdpError)`.  If the error
    //        signals a missing helper we fall back, otherwise we bubble it.
    //     c) The script succeeded but returned no numeric value.
    // -----------------------------------------------------------------
    let eval_res = match eval_outcome {
        Ok(res) => res,
        Err(err) => {
            if is_missing_helper_error(&err) {
                #[cfg(feature = "gemini")]
                {
                    let api_key = std::env::var("GEMINI_API_KEY")
                        .map_err(|_| CdpError::msg("GEMINI_API_KEY not set"))?;
                    return gemini::solve_with_gemini(&api_key, canvas_dataurl)
                        .await
                        .map_err(|e| CdpError::msg(format!("Gemini external error: {e}")));
                }

                #[cfg(not(feature = "gemini"))]
                {
                    // No Gemini compiled – return centre of track.
                    return Ok(0.0);
                }
            } else {
                // Some other Chrome‑side error – propagate it.
                return Err(err);
            }
        }
    };

    let maybe_offset = match eval_res.value() {
        Some(v) => match v {
            serde_json::Value::Number(n) => n.as_f64(),
            serde_json::Value::String(s) => s.parse::<f64>().ok(),
            _ => None,
        },
        None => None,
    };

    if let Some(off) = maybe_offset {
        return Ok(off);
    }

    Err(CdpError::msg(
        "In‑page Gemini helper returned no numeric result",
    ))
}

/// Geetest solving
#[cfg(all(feature = "chrome", feature = "real_browser"))]
#[inline(always)]
pub async fn geetest_handle(
    b: &mut Vec<u8>,
    page: &Page,
    viewport: &Option<crate::configuration::Viewport>,
) -> Result<bool, CdpError> {
    // -----------------------------------------------------------------
    // Fast gate – bail out early if the page does not look like GeeTest.
    // -----------------------------------------------------------------
    if !looks_like_geetest(b.as_slice()) {
        return Ok(false);
    }

    let mut progressed = false;

    // -----------------------------------------------------------------
    // Whole routine lives inside a 30 s timeout (same pattern as the rest
    // of the code‑base).
    // -----------------------------------------------------------------
    let page_result = tokio::time::timeout(Duration::from_secs(30), async {
        // Disable the network cache + a little “human” mouse movement.
        let _ = tokio::join!(
            page.disable_network_cache(true),
            perform_smart_mouse_movement(page, viewport)
        );

        for _ in 0..10 {
            // -------------------------------------------------------------
            // a) Refresh the HTML source.
            // -------------------------------------------------------------
            if let Ok(cur) = page.outer_html_bytes().await {
                *b = cur;
            }

            // -------------------------------------------------------------
            // b) If GeeTest vanished → success.
            // -------------------------------------------------------------
            if !looks_like_geetest(b.as_slice()) {
                progressed = true;
                break;
            }

            // -------------------------------------------------------------
            // c) Still loading?  Wait like Cloudflare.
            // -------------------------------------------------------------
            if looks_like_geetest_loading(b.as_slice()) {
                let mut wait_for = CF_WAIT_FOR.clone();
                wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                    Duration::from_millis(1_000),
                ))
                .into();
                wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                    Duration::from_secs(7).into(),
                )
                .into();
                wait_for.page_navigations = true;
                let wait = Some(wait_for.clone());
                let _ = tokio::join!(
                    page_wait(page, &wait),
                    perform_smart_mouse_movement(page, viewport),
                );
                continue;
            }

            // -------------------------------------------------------------
            // d) Click the “Click to verify” radar.
            // -------------------------------------------------------------
            let mut clicked = false;
            if let Ok(els) = page.find_elements_pierced(r#".geetest_radar"#).await {
                if let Some(el) = els.into_iter().next() {
                    clicked = match el.clickable_point().await {
                        Ok(p) => page.click(p).await.is_ok() || el.click().await.is_ok(),
                        Err(_) => el.click().await.is_ok(),
                    };
                }
            }
            // Fallback element.
            if !clicked {
                if let Ok(els) = page
                    .find_elements_pierced(r#".geetest_radar_tip_content"#)
                    .await
                {
                    if let Some(el) = els.into_iter().next() {
                        clicked = match el.clickable_point().await {
                            Ok(p) => page.click(p).await.is_ok() || el.click().await.is_ok(),
                            Err(_) => el.click().await.is_ok(),
                        };
                    }
                }
            }

            // -------------------------------------------------------------
            // e) Short wait after the click so the widget can render.
            // -------------------------------------------------------------
            let mut wait_for = CF_WAIT_FOR.clone();
            wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(if clicked {
                Duration::from_millis(900)
            } else {
                Duration::from_millis(700)
            }))
            .into();
            wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                Duration::from_secs(6).into(),
            )
            .into();
            wait_for.page_navigations = true;
            let wait = Some(wait_for.clone());
            let _ = tokio::join!(
                page_wait(page, &wait),
                perform_smart_mouse_movement(page, viewport),
            );

            // -------------------------------------------------------------
            // f) Refresh HTML again – now the slider should be visible.
            // -------------------------------------------------------------
            if let Ok(nc) = page.outer_html_bytes().await {
                *b = nc;

                if looks_like_geetest_challenge_visible(b.as_slice()) {
                    // -------------------------------------------------
                    //   🎯  ***  SOLVE THE SLIDER  ***  🎯
                    // -------------------------------------------------
                    // 1️⃣  Grab the *track* (the gray bar the button slides on)
                    //     and the slider button.
                    //     Try the v3 selectors first; fall back to the v4 ones.
                    // -------------------------------------------------
                    async fn first_of(
                        page: &Page,
                        sel_a: &str,
                        sel_b: &str,
                    ) -> Result<chromiumoxide::Element, CdpError> {
                        // Try selector A.
                        if let Ok(els) = page.find_elements_pierced(sel_a).await {
                            if let Some(el) = els.into_iter().next() {
                                return Ok(el);
                            }
                        }
                        // Fallback to selector B.
                        let els = page.find_elements_pierced(sel_b).await?;
                        let el = els.into_iter().next().ok_or_else(|| {
                            CdpError::msg(format!("neither {sel_a} nor {sel_b} found"))
                        })?;
                        Ok(el)
                    }

                    // Track – v3: .geetest_slicebg  |  v4: .geetest_wrap
                    let track_el = first_of(page, ".geetest_slicebg", ".geetest_wrap").await?;
                    let track_bb = track_el.bounding_box().await?;

                    // Button – v3: .geetest_slider_button  |  v4: .geetest_btn
                    let btn_el = first_of(page, ".geetest_slider_button", ".geetest_btn").await?;
                    let btn_bb = btn_el.bounding_box().await?;

                    // -------------------------------------------------
                    // 2️⃣  Locate the *canvas* that holds the puzzle image.
                    // -------------------------------------------------
                    let canvas_el = page
                        .find_elements_pierced(r#".geetest_canvas_slice.geetest_absolute"#)
                        .await?
                        .into_iter()
                        .next()
                        .ok_or_else(|| CdpError::msg("canvas element not found"))?;

                    // -------------------------------------------------
                    // 3️⃣  Pull the canvas data‑URL using the element we just
                    //     fetched (no unused‑variable warning).
                    // -------------------------------------------------
                    let dataurl: String = {
                        let call = CallFunctionOnParams::builder()
                            .object_id(canvas_el.remote_object_id.clone())
                            .function_declaration("(function(){ return this.toDataURL(); })")
                            .await_promise(true)
                            .build()
                            .unwrap();

                        // `page.evaluate_function` returns an `EvaluationResult`.
                        let eval_res = page.evaluate_function(call).await?;
                        eval_res
                            .value()
                            .and_then(|v| v.as_str().map(|s| s.to_owned()))
                            .ok_or_else(|| {
                                CdpError::msg("Failed to extract data‑url from canvas")
                            })?
                    };

                    // -------------------------------------------------
                    // 4️⃣  Try the in‑page Gemini helper first.  If it does not
                    //     exist we fall back to the external Gemini API (or the
                    //     centre‑of‑track when the gemini feature is disabled).
                    // -------------------------------------------------
                    let gap_x = match solve_geetest_with_inpage_helper(page, &dataurl, 20_000).await
                    {
                        Ok(x) => x,
                        Err(e) if is_missing_helper_error(&e) => {
                            #[cfg(feature = "gemini")]
                            {
                                let api_key = std::env::var("GEMINI_API_KEY")
                                    .map_err(|_| CdpError::msg("GEMINI_API_KEY not set"))?;
                                gemini::solve_with_gemini(&api_key, &dataurl)
                                    .await
                                    .map_err(|e| {
                                        CdpError::msg(format!("Gemini external error: {e}"))
                                    })?
                            }

                            #[cfg(not(feature = "gemini"))]
                            {
                                // centre of the track – old hard‑coded fallback.
                                (track_bb.width * 0.5) as f64
                            }
                        }
                        Err(e) => return Err(e), // real Chrome error – bubble up
                    };

                    // -------------------------------------------------
                    // 5️⃣  Convert the canvas‑relative offset into a *page*
                    //     coordinate.
                    // -------------------------------------------------
                    let canvas_width: f64 = page
                        .evaluate(format!(
                            "document.querySelector('{}').width",
                            ".geetest_canvas_slice.geetest_absolute"
                        ))
                        .await?
                        .into_value()?;

                    let proportion = (gap_x / canvas_width).clamp(0.0, 1.0);
                    let target_x = track_bb.x + proportion * track_bb.width;

                    // -------------------------------------------------
                    // 6️⃣  Build the drag points.
                    // -------------------------------------------------
                    let from = Point {
                        x: btn_bb.x + btn_bb.width * 0.5,
                        y: btn_bb.y + btn_bb.height * 0.5,
                    };
                    let to = Point {
                        x: target_x,
                        y: track_bb.y + track_bb.height * 0.5,
                    };

                    // -------------------------------------------------
                    // 7️⃣  Perform the drag.
                    // -------------------------------------------------
                    let _ = page.click_and_drag(from, to).await;

                    // -------------------------------------------------
                    // 8️⃣  Wait a little, then verify whether the widget vanished.
                    // -------------------------------------------------
                    let mut wf = CF_WAIT_FOR.clone();
                    wf.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                        Duration::from_millis(1_100),
                    ))
                    .into();
                    wf.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                        Duration::from_secs(7).into(),
                    )
                    .into();
                    wf.page_navigations = true;
                    let wait = Some(wf.clone());
                    let _ = tokio::join!(
                        page_wait(page, &wait),
                        perform_smart_mouse_movement(page, viewport),
                    );

                    // Refresh the HTML one final time.
                    if let Ok(nc2) = page.outer_html_bytes().await {
                        *b = nc2;
                        if !looks_like_geetest(b.as_slice()) {
                            progressed = true;
                            break;
                        }
                    }

                    // If we are still here the slider failed – loop again (max 10).
                    continue;
                }

                // If the widget disappeared after any step, we are done.
                if !looks_like_geetest(b.as_slice()) {
                    progressed = true;
                    break;
                }
            }
        }

        Ok::<(), CdpError>(())
    })
    .await;

    match page_result {
        Ok(_) => Ok(progressed),
        Err(_) => Err(CdpError::Timeout),
    }
}
