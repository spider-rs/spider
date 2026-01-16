use aho_corasick::AhoCorasick;
#[cfg(all(feature = "chrome", feature = "real_browser"))]
use chromiumoxide::{
    cdp::js_protocol::runtime::EvaluateParams, error::CdpError, layout::Point, Page,
};
use std::time::Duration;

#[cfg(all(feature = "chrome", feature = "real_browser"))]
use crate::utils::{page_wait, perform_smart_mouse_movement, RequestError, CF_WAIT_FOR};

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

lazy_static! {
    /// Gemini client
    static ref GEMINI_CLIENT: crate::Client = {
        crate::Client::builder()
                .timeout(Duration::from_millis(20_000))
                .build()
                .expect("client")
    };
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
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// CF prefix scan bytes.
const CF_PREFIX_SCAN_BYTES: usize = 120;

#[cfg(all(feature = "chrome", feature = "real_browser"))]
#[inline(always)]
/// CF slice prefix.
fn cf_prefix_slice(b: &[u8]) -> &[u8] {
    if b.len() > CF_PREFIX_SCAN_BYTES {
        &b[..CF_PREFIX_SCAN_BYTES]
    } else {
        b
    }
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
lazy_static! {
    static ref CF_END: &'static [u8; 62] =
        b"target=\"_blank\">Cloudflare</a></div></div></div></body></html>";
    static ref CF_END2: &'static [u8; 72] =
        b"Performance &amp; security by Cloudflare</div></div></div></body></html>";
    static ref CF_HEAD: &'static [u8; 34] = b"<html><head>\n    <style global=\"\">";
    static ref CF_MOCK_FRAME: &'static [u8; 137] = b"<iframe height=\"1\" width=\"1\" style=\"position: absolute; top: 0px; left: 0px; border: none; visibility: hidden;\"></iframe>\n\n</body></html>";
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

#[cfg(all(feature = "chrome", feature = "real_browser"))]
#[inline]
/// Is turnstile page? This does nothing without the 'real_browser' feature enabled.
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


  /// Empty HTML.
  pub static ref EMPTY_HTML: &'static [u8; 39] = b"<html><head></head><body></body></html>";
  /// Empty html.
  pub static ref EMPTY_HTML_BASIC: &'static [u8; 13] = b"<html></html>";
  /// The vision endpoint gemini.
  pub static ref GEMINI_VISION_ENDPOINT: String = {
    std::env::var("GEMINI_VISION_ENDPOINT").unwrap_or("https://generativelanguage.googleapis.com/v1beta/models/gemini-pro-vision".into())
  };
}

/// “Challenge-sized” heuristic.
///
/// Tune this threshold as you see real traffic:
/// - Imperva / captcha pages are often small HTML shells.
/// - Real pages can also be small, so we ALSO require iframe signatures.
#[inline(always)]
pub fn imperva_challenge_sized(content_len: usize) -> bool {
    // keep it branch-light; adjust if needed
    content_len > 0 && content_len <= 200_000
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
        let _ = tokio::join!(page.disable_network_cache(true), page_navigate, perform_smart_mouse_movement(&page, &viewport));

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
                perform_smart_mouse_movement(&page, &viewport).await;
                for el in els {
                    let f = async {
                        match el.clickable_point().await {
                            Ok(pt) => page.click(pt).await.is_ok() || el.click().await.is_ok(),
                            Err(_) => el.click().await.is_ok(),
                        }
                    };

                    let (did_click, _) =
                        tokio::join!(f, perform_smart_mouse_movement(&page, &viewport));

                    if did_click {
                        clicks += 1;
                    }
                }
            } else {

                hidden = true;
                let wait = Some(wait_for.clone());
                let _ = tokio::join!(
                    page_wait(&page, &wait),
                    perform_smart_mouse_movement(&page, &viewport)
                );
            }

            if !hidden && clicks == 0 {
                let f = page.evaluate(
                    r#"document.querySelectorAll("iframe,input")?.forEach(el => el.click());document.querySelector('.cf-turnstile')?.click();"#,
                );
                let _ = tokio::join!(f, perform_smart_mouse_movement(&page, &viewport));
            }

            wait_for.page_navigations = true;
            let wait = Some(wait_for.clone());

            let _ = tokio::join!(
                page_wait(&page, &wait),
                perform_smart_mouse_movement(&page, &viewport),
            );

            if let Ok(mut next_content) = page.outer_html_bytes().await {
                if !detect_cf_turnstyle(&next_content) {
                    validated = true;
                    wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                        core::time::Duration::from_secs(4),
                    ))
                    .into();
                    page_wait(&page, &Some(wait_for)).await;
                    if let Ok(nc) = page.outer_html_bytes().await {
                        next_content = nc;
                    }
                } else if contains_verification(&next_content) {
                    wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                        core::time::Duration::from_millis(3500),
                    ))
                    .into();
                    page_wait(&page, &Some(wait_for.clone())).await;

                    if let Ok(nc) = page.outer_html_bytes().await {
                        next_content = nc;
                    }
                    if !detect_cf_turnstyle(&next_content) {
                        validated = true;
                        page_wait(&page, &Some(wait_for)).await;
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

/// Handle protected pages via chrome. This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", not(feature = "real_browser")))]
#[inline(always)]
pub async fn cf_handle(
    _b: &mut Vec<u8>,
    _page: &chromiumoxide::Page,
    _target_url: &str,
    _viewport: &Option<crate::configuration::Viewport>,
) -> Result<(), chromiumoxide::error::CdpError> {
    Ok(())
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
    // ------------------------------------------------------------
    // Fast, no-alloc detection helpers (best perf)
    // ------------------------------------------------------------
    #[inline(always)]
    fn imperva_challenge_sized(len: usize) -> bool {
        len > 0 && len <= 220_000
    }

    #[inline(always)]
    fn memeq(h: &[u8], n: &[u8]) -> bool {
        h.len() == n.len() && h.iter().zip(n).all(|(a, b)| a == b)
    }

    #[inline(always)]
    fn contains(h: &[u8], needle: &[u8]) -> bool {
        let nl = needle.len();
        if nl == 0 {
            return true;
        }
        if nl > h.len() {
            return false;
        }
        h.windows(nl).any(|w| memeq(w, needle))
    }

    // Wait screen like your screenshot.
    #[inline(always)]
    fn looks_like_imperva_wait_screen(html: &[u8]) -> bool {
        if !imperva_challenge_sized(html.len()) {
            return false;
        }
        contains(html, b"Verifying the device")
            || contains(html, b"Verifying the device...")
            || contains(
                html,
                b"The requested content will be available after verification",
            )
            || contains(html, b"available after verification")
    }

    // Imperva iframe/verification system present (slider phase).
    #[inline(always)]
    fn looks_like_imperva_iframe_phase(html: &[u8]) -> bool {
        if !imperva_challenge_sized(html.len()) {
            return false;
        }
        contains(html, b"geo.captcha-delivery.com")
            || contains(html, b"captcha-delivery.com")
            || contains(html, b"Verification system")
    }

    // hCaptcha iframe present (pokemoncenter appears to do this)
    #[inline(always)]
    fn looks_like_hcaptcha_iframe(html: &[u8]) -> bool {
        if !imperva_challenge_sized(html.len()) {
            return false;
        }
        // typical iframe src host(s) + keyword
        contains(html, b"newassets.hcaptcha.com")
            || contains(html, b"hcaptcha.com/captcha")
            || contains(html, b"Widget containing checkbox for hCaptcha")
            || contains(html, b"data-hcaptcha-widget-id")
    }

    #[inline(always)]
    fn looks_like_imperva_any(html: &[u8]) -> bool {
        looks_like_imperva_wait_screen(html)
            || looks_like_imperva_iframe_phase(html)
            || looks_like_hcaptcha_iframe(html)
    }

    // If the caller buffer doesn't look like Imperva challenge at all, do nothing.
    if !looks_like_imperva_any(b.as_slice()) {
        return Ok(false);
    }

    // ------------------------------------------------------------
    // Drag helpers
    // ------------------------------------------------------------
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

    // JS drag builder (fast): 1 String alloc + one write!
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

    // ------------------------------------------------------------
    // Main loop
    // ------------------------------------------------------------
    let mut validated = false;

    let page_result = tokio::time::timeout(tokio::time::Duration::from_secs(30), async {
        let _ = tokio::join!(
            page.disable_network_cache(true),
            perform_smart_mouse_movement(&page, &viewport)
        );

        for _ in 0..10 {
            let mut wait_for = CF_WAIT_FOR.clone();

            // Pull HTML once per iteration.
            let cur_html = match page.outer_html_bytes().await {
                Ok(h) => h,
                Err(_) => {
                    let wait = Some(wait_for.clone());
                    let _ = tokio::join!(
                        page_wait(&page, &wait),
                        perform_smart_mouse_movement(&page, &viewport),
                    );
                    continue;
                }
            };

            *b = cur_html;

            // If we left the challenge, done.
            if !looks_like_imperva_any(b.as_slice()) {
                validated = true;
                break;
            }

            // ------------------------------------------------------------
            // 0) hCaptcha checkbox flow
            // ------------------------------------------------------------
            // If an hCaptcha iframe is present, click the checkbox with id="checkbox" inside it.
            // Keep it simple: pierce into iframes and click #checkbox; then wait a little.
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
                        // Click the checkbox. Prefer clickable_point if available.
                        let clicked = match cb_el.clickable_point().await {
                            Ok(p) => page.click(p).await.is_ok() || cb_el.click().await.is_ok(),
                            Err(_) => cb_el.click().await.is_ok(),
                        };

                        if clicked {
                            // Give it a moment to render/transition.
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
                                page_wait(&page, &wait),
                                perform_smart_mouse_movement(&page, &viewport),
                            );

                            if let Ok(nc) = page.outer_html_bytes().await {
                                *b = nc;
                                if !looks_like_imperva_any(b.as_slice()) {
                                    validated = true;
                                    break;
                                }
                            }
                        } else {
                            // Even if click failed, wait a bit; sometimes it becomes clickable after.
                            wait_for.delay = crate::features::chrome_common::WaitForDelay::new(
                                Some(core::time::Duration::from_millis(650)),
                            )
                            .into();
                            let wait = Some(wait_for.clone());
                            let _ = tokio::join!(
                                page_wait(&page, &wait),
                                perform_smart_mouse_movement(&page, &viewport),
                            );
                        }

                        // Continue loop; might transition into slider phase or pass.
                        continue;
                    }
                }

                // If we didn't find #checkbox yet, act like CF: wait for iframe content to load.
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
                    page_wait(&page, &wait),
                    perform_smart_mouse_movement(&page, &viewport),
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

            // ------------------------------------------------------------
            // 1) WAIT SCREEN (no iframe yet): wait until it progresses
            // ------------------------------------------------------------
            if looks_like_imperva_wait_screen(b.as_slice()) {
                wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                    core::time::Duration::from_millis(1100),
                ))
                .into();
                wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(
                    core::time::Duration::from_secs(7).into(),
                )
                .into();
                wait_for.page_navigations = true;

                let wait = Some(wait_for.clone());
                let _ = tokio::join!(
                    page_wait(&page, &wait),
                    perform_smart_mouse_movement(&page, &viewport),
                );

                if let Ok(nc) = page.outer_html_bytes().await {
                    *b = nc;
                }
                continue;
            }

            // ------------------------------------------------------------
            // 2) Imperva iframe/slider phase: drag
            // ------------------------------------------------------------
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

                // Prefer handle drag if present
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
                    if let Some(h) = handles.into_iter().next() {
                        if let (Ok(hb), Ok(conts)) = (
                            h.bounding_box().await,
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
                            if let Some(c) = conts.into_iter().next() {
                                if let Ok(cb) = c.bounding_box().await {
                                    let from = pt(hb.x + hb.width * 0.5, hb.y + hb.height * 0.5);

                                    let to_x = cb.x + cb.width - 8.0;
                                    let to_y = cb.y + cb.height * 0.5;
                                    let to = pt(
                                        clamp(to_x, cb.x + 2.0, cb.x + cb.width - 2.0),
                                        clamp(to_y, cb.y + 2.0, cb.y + cb.height - 2.0),
                                    );

                                    let _ = tokio::join!(
                                        perform_smart_mouse_movement(&page, &viewport),
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

                // JS fallback using container bbox
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
                        if let Some(c) = conts.into_iter().next() {
                            if let Ok(cb) = c.bounding_box().await {
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

                // Wait after interaction
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
                    page_wait(&page, &wait),
                    perform_smart_mouse_movement(&page, &viewport),
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

            // ------------------------------------------------------------
            // 3) Unknown interstitial: do a CF-style wait and re-check.
            // ------------------------------------------------------------
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
                page_wait(&page, &wait),
                perform_smart_mouse_movement(&page, &viewport),
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

/// Handle imperva protected pages via chrome. This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", not(feature = "real_browser")))]
#[inline(always)]
pub async fn imperva_handle(
    _b: &mut Vec<u8>,
    _page: &chromiumoxide::Page,
    _target_url: &str,
    _viewport: &Option<crate::configuration::Viewport>,
) -> Result<(), chromiumoxide::error::CdpError> {
    Ok(())
}

/// Calls the in‑page JS helper defined above and returns the ids that the model said “yes”.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
pub async fn solve_enterprise_with_browser_gemini(
    page: &chromiumoxide::Page,
    challenge: &RcEnterpriseChallenge<'_>,
    timeout_ms: u64,
) -> Result<Vec<u8>, CdpError> {
    match solve_with_inpage_helper(page, challenge, timeout_ms).await {
        Ok(ids) => return Ok(ids),
        Err(e) if !is_missing_helper_error(&e) => return Err(e),
        Err(_) => {}
    }

    solve_with_external_gemini(challenge, timeout_ms)
        .await
        .map_err(|e| CdpError::ChromeMessage(format!("external‑gemini failed: {e}")))
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
/// In page solve helper gemini nano solving challenge.
async fn solve_with_inpage_helper(
    page: &chromiumoxide::Page,
    challenge: &RcEnterpriseChallenge<'_>,
    timeout_ms: u64,
) -> Result<Vec<u8>, CdpError> {
    let tiles_json = challenge
        .tiles
        .iter()
        .map(|t| serde_json::json!({ "id": t.id, "src": t.img_src }))
        .collect::<Vec<_>>();

    let target = challenge.target.unwrap_or("target object").to_string();

    let script_template = r#"
        async function solveRecaptchaEnterpriseWithGemini(tiles, target, timeout) {
          return new Promise(async (resolve, reject) => {
            try {
              const session = await LanguageModel.create({
                expectedInputs: [
                  { type: "text", languages: ["en"] },
                  { type: "image" },
                ],
                expectedOutputs: [{ type: "text", languages: ["en"] }],
              });
              const yesIds = [];
              for (const tile of tiles) {
                const resp = await fetch(tile.src, { mode: "cors" });
                if (!resp.ok) continue;
                const blob = await resp.blob();
                const prompt = [
                  {
                    role: "user",
                    content: [
                      {
                        type: "text",
                        value: `Does this image contain a ${target}? Answer only with "yes" or "no".`,
                      },
                      { type: "image", value: blob },
                    ],
                  },
                ];
                const answer = await session.prompt(prompt);
                const txt = (answer || "").toString().trim().toLowerCase();
                if (txt.includes("yes")) {
                  yesIds.push(tile.id);
                }
              }
              resolve(yesIds);
            } catch (e) {
              reject(e);
            }
          });
        }

        (async () => {
          const result = await solveRecaptchaEnterpriseWithGemini(
            %tiles%,
            %target%,
            %timeout%
          );
          return result;
        })()
    "#;

    let script = script_template
        .replace("%tiles%", &serde_json::to_string(&tiles_json).unwrap())
        .replace("%target%", &serde_json::to_string(&target).unwrap())
        .replace("%timeout%", &timeout_ms.to_string());

    // ---------- 3️⃣  Ask Chrome to evaluate ----------
    let params = chromiumoxide::cdp::js_protocol::runtime::EvaluateParams::builder()
        .expression(script)
        .await_promise(true)
        .build()
        .unwrap();

    // The Chrome call itself may time‑out; we give it a little extra margin.
    let eval_fut = page.evaluate(params);
    let eval_res = tokio::time::timeout(Duration::from_millis(timeout_ms + 5_000), eval_fut).await;

    // -----------------------------------------------------------------
    // 4️⃣  Turn the JS result into a Vec<u8>.
    // -----------------------------------------------------------------
    match eval_res {
        Ok(Ok(eval)) => match eval.value() {
            Some(serde_json::Value::Array(arr)) => {
                let ids = arr
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u8))
                    .collect();
                Ok(ids)
            }
            _ => Ok(vec![]), // empty / not an array → nothing matched
        },
        // Chrome returned an error – we forward it as‑is; the caller decides
        // whether it is a “missing helper” situation.
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => Err(CdpError::Timeout),
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
                                    "data": base64::encode(&img_bytes)
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

/// Handle reCAPTCHA checkbox (anchor iframe) via chrome.
/// This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
#[inline(always)]
pub async fn recaptcha_handle(
    b: &mut Vec<u8>,
    page: &chromiumoxide::Page,
    viewport: &Option<crate::configuration::Viewport>,
) -> Result<bool, CdpError> {
    #[inline(always)]
    fn memeq(h: &[u8], n: &[u8]) -> bool {
        h.len() == n.len() && h.iter().zip(n).all(|(a, b)| a == b)
    }
    #[inline(always)]
    fn contains(h: &[u8], needle: &[u8]) -> bool {
        h.windows(needle.len()).any(|w| memeq(w, needle))
    }

    #[inline(always)]
    fn looks_like_any_recaptcha(html: &[u8]) -> bool {
        // classic markers
        let classic = contains(html, b"/recaptcha/api2/anchor")
            || contains(html, b"www.google.com/recaptcha/api2/anchor")
            || contains(html, b"reCAPTCHA")
            || contains(html, b"recaptcha-anchor");
        // enterprise markers (the same ones `extract_rc_enterprise_challenge` uses)
        let enterprise = contains(html, b"__recaptcha_api")
            && contains(html, b"/recaptcha/enterprise/")
            && contains(html, b"rc-imageselect")
            && contains(html, b"rc-imageselect-tile");
        classic || enterprise
    }

    // -----------------------------------------------------------------
    // Fast‑path – if we don’t see any Recaptcha at all, bail out.
    // -----------------------------------------------------------------
    if !looks_like_any_recaptcha(b.as_slice()) {
        return Ok(false);
    }

    // -----------------------------------------------------------------
    // Main loop (≤10 attempts, 30 s total timeout).
    // -----------------------------------------------------------------
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
            if !looks_like_any_recaptcha(b.as_slice()) {
                validated = true;
                break;
            }

            // ---------------------------------------------------------
            // c) **Enterprise** handling – now solved with the built‑in Gemini.
            // ---------------------------------------------------------
            if let Some(_) = extract_rc_enterprise_challenge(b.as_slice()) {
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
                        page_wait(&page, &wait),
                        perform_smart_mouse_movement(&page, viewport),
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
                    page_wait(&page, &wait),
                    perform_smart_mouse_movement(&page, viewport),
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
                if !looks_like_any_recaptcha(b.as_slice()) {
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
                    page_wait(&page, &wait),
                    perform_smart_mouse_movement(&page, viewport),
                );

                // ---------------------------------------------------------
                // i) Refresh HTML one last time – if the whole Recaptcha is gone we’re finished.
                // ---------------------------------------------------------
                if let Ok(new_html) = page.outer_html_bytes().await {
                    *b = new_html;
                    if !looks_like_any_recaptcha(b.as_slice()) {
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
                    page_wait(&page, &wait),
                    perform_smart_mouse_movement(&page, viewport),
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
                page_wait(&page, &wait),
                perform_smart_mouse_movement(&page, viewport),
            );

            if let Ok(new_html) = page.outer_html_bytes().await {
                *b = new_html;
                if !looks_like_any_recaptcha(b.as_slice()) {
                    validated = true;
                    break;
                }
            }
        }

        Ok::<(), CdpError>(())
    })
    .await;

    // -----------------------------------------------------------------
    // Propagate the result exactly like the original implementation.
    // -----------------------------------------------------------------
    match overall {
        Ok(_) => Ok(validated),
        Err(_) => Err(CdpError::Timeout),
    }
}

/// Handle reCAPTCHA checkbox (anchor iframe) via chrome.
/// This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", not(feature = "real_browser")))]
#[inline(always)]
async fn recaptcha_handle(
    _b: &mut Vec<u8>,
    _page: &chromiumoxide::Page,
    _target_url: &str,
    _viewport: &Option<crate::configuration::Viewport>,
) -> Result<(), chromiumoxide::error::CdpError> {
    Ok(())
}

/// Handle GeeTest presence via chrome (detect + wait + open widget).
#[cfg(all(feature = "chrome", not(feature = "real_browser")))]
#[inline(always)]
async fn geetest_handle(
    _b: &mut Vec<u8>,
    _page: &chromiumoxide::Page,
    _target_url: &str,
    _viewport: &Option<crate::configuration::Viewport>,
) -> Result<(), chromiumoxide::error::CdpError> {
    Ok(())
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

/// Extracts recaptcha enterprise image-grid metadata from the iframe inner HTML.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
#[inline(always)]
pub fn extract_rc_enterprise_challenge<'a>(html: &'a [u8]) -> Option<RcEnterpriseChallenge<'a>> {
    #[inline(always)]
    fn memeq(a: &[u8], b: &[u8]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y)
    }
    #[inline(always)]
    fn find(h: &[u8], needle: &[u8], start: usize) -> Option<usize> {
        let nl = needle.len();
        if nl == 0 || start >= h.len() || nl > (h.len() - start) {
            return None;
        }
        h[start..]
            .windows(nl)
            .position(|w| memeq(w, needle))
            .map(|p| start + p)
    }
    #[inline(always)]
    fn contains(h: &[u8], needle: &[u8]) -> bool {
        find(h, needle, 0).is_some()
    }
    #[inline(always)]
    fn find_quote_end(h: &[u8], start: usize) -> Option<usize> {
        h.get(start..)?
            .iter()
            .position(|&c| c == b'"')
            .map(|p| start + p)
    }
    #[inline(always)]
    fn is_digit(b: u8) -> bool {
        (b'0'..=b'9').contains(&b)
    }
    #[inline(always)]
    fn parse_u8_1digit(b: u8) -> Option<u8> {
        if is_digit(b) {
            Some(b - b'0')
        } else {
            None
        }
    }

    // Quick gate: must look like enterprise iframe + image grid.
    if !contains(html, b"__recaptcha_api")
        || !contains(html, b"/recaptcha/enterprise/")
        || !contains(html, b"rc-imageselect")
        || !contains(html, b"rc-imageselect-tile")
    {
        return None;
    }

    let mut out = RcEnterpriseChallenge {
        target: None,
        instruction_text: None,
        tiles: Vec::with_capacity(12),
        has_verify_button: contains(html, b"id=\"recaptcha-verify-button\"")
            || contains(html, b">Verify<"),
    };

    // ----------------------------
    // 1) Extract target word from:
    //    Select all images with <strong>bridges</strong>
    // ----------------------------
    // Primary: grab inside `<strong ...>WORD</strong>` near rc-imageselect-desc
    const DESC_PAT: &[u8] = b"rc-imageselect-desc";
    const STRONG_OPEN: &[u8] = b"<strong";
    const GT: &[u8] = b">";
    const STRONG_CLOSE: &[u8] = b"</strong>";

    if let Some(desc_pos) = find(html, DESC_PAT, 0) {
        // scan forward in a bounded window for strong tag
        let win_end = (desc_pos + 900).min(html.len());
        if let Some(strong_pos) = find(html, STRONG_OPEN, desc_pos) {
            if strong_pos < win_end {
                if let Some(gt_pos) = find(html, GT, strong_pos) {
                    let text_start = gt_pos + 1;
                    if let Some(close_pos) = find(html, STRONG_CLOSE, text_start) {
                        if close_pos <= win_end {
                            if let Ok(word) = core::str::from_utf8(&html[text_start..close_pos]) {
                                // trim whitespace
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

        // Optional: extract the whole desc text node (cheap-ish)
        // Find the first '>' after rc-imageselect-desc and then '<' end.
        if let Some(tag_end) = find(html, b">", desc_pos) {
            let t0 = tag_end + 1;
            // find first '<' after that
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

    // ----------------------------
    // 2) Extract tiles:
    //    <td ... id="0" class="rc-imageselect-tile" ...>
    //    <img ... src="https://www.google.com/recaptcha/enterprise/payload?...">
    // ----------------------------
    const TILE_CLASS: &[u8] = b"rc-imageselect-tile";
    const ID_PAT: &[u8] = b"id=\"";
    const SRC_PAT: &[u8] = b"src=\"";
    const PAYLOAD_PREFIX: &[u8] = b"https://www.google.com/recaptcha/enterprise/payload";

    let mut i = 0usize;
    while i < html.len() {
        let tile_pos = match find(html, TILE_CLASS, i) {
            Some(p) => p,
            None => break,
        };

        // bounded backscan to find id="X" close to this tile
        let back = tile_pos.saturating_sub(240);
        let id_pos = match find(html, ID_PAT, back) {
            Some(p) if p < tile_pos => p,
            _ => {
                i = tile_pos + TILE_CLASS.len();
                continue;
            }
        };

        let id = match html
            .get(id_pos + ID_PAT.len())
            .copied()
            .and_then(parse_u8_1digit)
        {
            Some(v) => v,
            None => {
                i = tile_pos + TILE_CLASS.len();
                continue;
            }
        };

        // find img src after tile_pos
        let src_pos = match find(html, SRC_PAT, tile_pos) {
            Some(p) => p,
            None => {
                i = tile_pos + TILE_CLASS.len();
                continue;
            }
        };

        let url_start = src_pos + SRC_PAT.len();
        if html.get(url_start..url_start + PAYLOAD_PREFIX.len()) != Some(PAYLOAD_PREFIX) {
            i = tile_pos + TILE_CLASS.len();
            continue;
        }

        let url_end = match find_quote_end(html, url_start) {
            Some(e) => e,
            None => {
                i = tile_pos + TILE_CLASS.len();
                continue;
            }
        };

        let url = match core::str::from_utf8(&html[url_start..url_end]) {
            Ok(s) => s,
            Err(_) => {
                i = tile_pos + TILE_CLASS.len();
                continue;
            }
        };

        // dedupe by id (recaptcha can re-render)
        if !out.tiles.iter().any(|t| t.id == id) {
            out.tiles.push(RcTileRef { id, img_src: url });
        }

        i = url_end + 1;
    }

    if out.tiles.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Gemini  helper.
#[cfg(feature = "gemini")]
mod gemini {
    use super::*;
    use anyhow::anyhow;
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
    pub async fn solve_with_gemini(api_key: &str, image_dataurl: &str) -> anyhow::Result<f64> {
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
                    let api_key = env::var("GEMINI_API_KEY")
                        .map_err(|_| CdpError::msg("GEMINI_API_KEY not set"))?;
                    return crate::gemini::solve_with_gemini(&api_key, canvas_dataurl)
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
    #[inline(always)]
    fn memeq(h: &[u8], n: &[u8]) -> bool {
        h.len() == n.len() && h.iter().zip(n).all(|(a, b)| a == b)
    }
    #[inline(always)]
    fn contains(h: &[u8], needle: &[u8]) -> bool {
        let nl = needle.len();
        if nl == 0 {
            return true;
        }
        if nl > h.len() {
            return false;
        }
        h.windows(nl).any(|w| memeq(w, needle))
    }
    #[inline(always)]
    fn looks_like_geetest(html: &[u8]) -> bool {
        contains(html, b"api.geetest.com")
            || contains(html, b"static.geetest.com")
            || contains(html, b"geetest_")
            || contains(html, b"Loading GeeTest")
            || contains(html, b"geetest_radar")
            || contains(html, b"geetest_widget")
            || contains(html, b"geetest_slider")
    }
    #[inline(always)]
    fn looks_like_geetest_loading(html: &[u8]) -> bool {
        contains(html, b"Loading GeeTest")
            || contains(html, b"geetest_wait")
            || contains(html, b"geetest_init")
    }
    #[inline(always)]
    fn looks_like_geetest_challenge_visible(html: &[u8]) -> bool {
        contains(html, b"geetest_widget")
            || contains(html, b"geetest_slider_button")
            || contains(html, b"geetest_canvas")
            || contains(html, b"geetest_canvas_slice")
    }

    // -----------------------------------------------------------------
    //  Gate – bail out early if the page does not look like GeeTest.
    // -----------------------------------------------------------------
    if !looks_like_geetest(b.as_slice()) {
        return Ok(false);
    }

    let mut progressed = false;

    // -----------------------------------------------------------------
    //  Whole routine lives inside a 30 s timeout (same as before).
    // -----------------------------------------------------------------
    let page_result = tokio::time::timeout(Duration::from_secs(30), async {
        // Disable the network cache + a little “human” mouse movement.
        let _ = tokio::join!(
            page.disable_network_cache(true),
            perform_smart_mouse_movement(page, viewport)
        );

        for _ in 0..10 {
            // -------------------------------------------------------------
            //   a) Refresh the HTML source.
            // -------------------------------------------------------------
            if let Ok(cur) = page.outer_html_bytes().await {
                *b = cur;
            }

            // -------------------------------------------------------------
            //   b) If GeeTest vanished → success.
            // -------------------------------------------------------------
            if !looks_like_geetest(b.as_slice()) {
                progressed = true;
                break;
            }

            // -------------------------------------------------------------
            //   c) Still loading?  Wait like Cloudflare.
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
            //   d) Click the “Click to verify” radar.
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
            //   e) Short wait after the click so the widget can render.
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
            //   f) Refresh HTML again – now the slider should be visible.
            // -------------------------------------------------------------
            if let Ok(nc) = page.outer_html_bytes().await {
                *b = nc;

                if looks_like_geetest_challenge_visible(b.as_slice()) {
                    // -------------------------------------------------
                    //   🎯  ***  SOLVE THE SLIDER  ***  🎯
                    // -------------------------------------------------
                    // 1️⃣  Grab the *track* (the gray bar the button slides on).
                    let (track_bb, btn_bb) = {
                        //   – track
                        let track_el = page
                            .find_elements_pierced(r#".geetest_slicebg"#)
                            .await?
                            .into_iter()
                            .next()
                            .ok_or_else(|| CdpError::msg("track element not found"))?;
                        let track_bb = track_el.bounding_box().await?;

                        //   – button
                        let btn_el = page
                            .find_elements_pierced(r#".geetest_slider_button"#)
                            .await?
                            .into_iter()
                            .next()
                            .ok_or_else(|| CdpError::msg("slider button not found"))?;
                        let btn_bb = btn_el.bounding_box().await?;

                        (track_bb, btn_bb)
                    };

                    // 2️⃣  Locate the *canvas* that holds the puzzle image.
                    let canvas_el = page
                        .find_elements_pierced(r#".geetest_canvas_slice.geetest_absolute"#)
                        .await?
                        .into_iter()
                        .next()
                        .ok_or_else(|| CdpError::msg("canvas element not found"))?;

                    // 3️⃣  Pull the canvas data‑URL.
                    //
                    //    `canvas.toDataURL()` returns something like
                    //    `data:image/png;base64,iVBORw0KGgoAAA…`
                    let js = r#"
                        (function(el){ return el.toDataURL(); })
                    "#;
                    let dataurl: String = page
                        .evaluate(format!(
                            "{}(document.querySelector('{}'))",
                            js, ".geetest_canvas_slice.geetest_absolute"
                        ))
                        .await?
                        .into_value()?;

                    // -------------------------------------------------
                    // 4️⃣  Try the *in‑page* helper first (the same pattern you have
                    //     for Recaptcha‑Enterprise).  If it does not exist we fall
                    //     back to the external Gemini call.
                    // -------------------------------------------------
                    let gap_x = match solve_geetest_with_inpage_helper(page, &dataurl, 20_000).await
                    {
                        Ok(x) => x,
                        Err(e) if is_missing_helper_error(&e) => {
                            // -----------------------------------------------------------------
                            //   4a) Helper missing – use the external Gemini model.
                            // -----------------------------------------------------------------
                            #[cfg(feature = "gemini")]
                            {
                                let api_key = env::var("GEMINI_API_KEY").map_err(|_| {
                                    CdpError::Message("GEMINI_API_KEY not set".into())
                                })?;
                                gemini::solve_with_gemini(&api_key, &dataurl)
                                    .await
                                    .map_err(|e| {
                                        CdpError::Message(format!("Gemini external error: {e}"))
                                    })?
                            }

                            // -----------------------------------------------------------------
                            //   4b) No Gemini feature – just fall back to the centre of the track.
                            // -----------------------------------------------------------------
                            #[cfg(not(feature = "gemini"))]
                            {
                                // centre of the track (same as the old “hard‑coded” drag)
                                (track_bb.width * 0.5) as f64
                            }
                        }
                        Err(e) => return Err(e), // real Chrome‑error, bubble up
                    };

                    // -------------------------------------------------
                    // 5️⃣  Convert the canvas‑relative offset into a *page* coordinate.
                    // -------------------------------------------------
                    // The canvas element reports its *intrinsic* width/height via HTML
                    // attributes – we read those attributes to know the scaling factor.
                    let canvas_width: f64 = page
                        .evaluate(format!(
                            "document.querySelector('{}').width",
                            ".geetest_canvas_slice.geetest_absolute"
                        ))
                        .await?
                        .into_value()?;

                    // Proportion of the gap inside the canvas (0.0‑1.0)
                    let proportion = (gap_x / canvas_width).clamp(0.0, 1.0);
                    // Where on the *track* we need to stop the drag.
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
