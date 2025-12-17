use crate::utils::trie::Trie;

#[cfg(feature = "chrome")]
use chromiumoxide::handler::blockers::NetworkInterceptManager;

/// wrapper for non chrome interception. does nothing.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[cfg(not(feature = "chrome"))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NetworkInterceptManager {
    #[default]
    /// Unknown
    Unknown,
}

#[cfg(not(feature = "chrome"))]
impl NetworkInterceptManager {
    /// a custom intercept handle.
    pub fn new(_url: &Option<Box<url::Url>>) -> NetworkInterceptManager {
        NetworkInterceptManager::Unknown
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Wait for network request. This does nothing without the `chrome` flag enabled.
pub struct WaitForIdleNetwork {
    /// The max time to wait for the network. It is recommended to set this to a value around 30s. Set the value to None to remove the timeout.
    pub timeout: Option<core::time::Duration>,
}

impl WaitForIdleNetwork {
    /// Create new WaitForIdleNetwork with timeout.
    pub fn new(timeout: Option<core::time::Duration>) -> Self {
        Self { timeout }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Wait for a selector with optional timeout. This does nothing without the `chrome` flag enabled.
pub struct WaitForSelector {
    /// The max time to wait for the selector. It is recommended to set this to a value around 30s. Set the value to None to remove the timeout.
    pub timeout: Option<core::time::Duration>,
    /// The selector wait for
    pub selector: String,
}

impl WaitForSelector {
    /// Create new WaitForSelector with timeout.
    pub fn new(timeout: Option<core::time::Duration>, selector: String) -> Self {
        Self { timeout, selector }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Wait for with a delay. Should only be used for testing purposes. This does nothing without the `chrome` flag enabled.
pub struct WaitForDelay {
    /// The max time to wait. It is recommended to set this to a value around 30s. Set the value to None to remove the timeout.
    pub timeout: Option<core::time::Duration>,
}

impl WaitForDelay {
    /// Create new WaitForDelay with timeout.
    pub fn new(timeout: Option<core::time::Duration>) -> Self {
        Self { timeout }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// The wait for options for the page. Multiple options can be set. This does nothing without the `chrome` flag enabled.
pub struct WaitFor {
    /// The max time to wait for the selector.
    pub selector: Option<WaitForSelector>,
    /// Wait for network request to be idle within a time frame period (500ms no network connections). This does nothing without the `chrome` flag enabled.
    pub idle_network: Option<WaitForIdleNetwork>,
    /// Wait for network request with a max timeout. This does nothing without the `chrome` flag enabled.
    pub idle_network0: Option<WaitForIdleNetwork>,
    /// Wait for network to be almost idle with a max timeout. This does nothing without the `chrome` flag enabled.
    pub almost_idle_network0: Option<WaitForIdleNetwork>,
    /// Wait for delay. Should only be used for testing.
    pub delay: Option<WaitForDelay>,
    /// Wait for dom element to stop updating.
    pub dom: Option<WaitForSelector>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Wait for page navigations.
    pub page_navigations: bool,
}

impl WaitFor {
    /// Create new WaitFor with timeout.
    pub fn new(
        timeout: Option<core::time::Duration>,
        delay: Option<WaitForDelay>,
        page_navigations: bool,
        idle_network: bool,
        selector: Option<String>,
        dom: Option<WaitForSelector>,
    ) -> Self {
        Self {
            page_navigations,
            idle_network: if idle_network {
                Some(WaitForIdleNetwork::new(timeout))
            } else {
                None
            },
            idle_network0: None,
            almost_idle_network0: None,
            selector: if selector.is_some() {
                Some(WaitForSelector::new(timeout, selector.unwrap_or_default()))
            } else {
                None
            },
            delay,
            dom,
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Default, strum::EnumString, strum::Display, strum::AsRefStr,
)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Capture screenshot options for chrome.
pub enum CaptureScreenshotFormat {
    #[cfg_attr(feature = "serde", serde(rename = "jpeg"))]
    /// jpeg format
    Jpeg,
    #[cfg_attr(feature = "serde", serde(rename = "png"))]
    #[default]
    /// png format
    Png,
    #[cfg_attr(feature = "serde", serde(rename = "webp"))]
    /// webp format
    Webp,
}

#[cfg(feature = "chrome")]
impl From<CaptureScreenshotFormat>
    for chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat
{
    fn from(format: CaptureScreenshotFormat) -> Self {
        match format {
            CaptureScreenshotFormat::Jpeg => {
                chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Jpeg
            }
            CaptureScreenshotFormat::Png => {
                chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png
            }
            CaptureScreenshotFormat::Webp => {
                chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Webp
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// View port handling for chrome.
pub struct Viewport {
    /// Device screen Width
    pub width: u32,
    /// Device screen size
    pub height: u32,
    /// Device scale factor
    pub device_scale_factor: Option<f64>,
    /// Emulating Mobile?
    pub emulating_mobile: bool,
    /// Use landscape mode instead of portrait.
    pub is_landscape: bool,
    /// Touch screen device?
    pub has_touch: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            width: 800,
            height: 600,
            device_scale_factor: None,
            emulating_mobile: false,
            is_landscape: false,
            has_touch: false,
        }
    }
}

impl Viewport {
    /// Create a new viewport layout for chrome passing in the width.
    pub fn new(width: u32, height: u32) -> Self {
        Viewport {
            width,
            height,
            ..Default::default()
        }
    }
    /// Determine if the layout is a mobile device or not to emulate.
    pub fn set_mobile(&mut self, emulating_mobile: bool) {
        self.emulating_mobile = emulating_mobile;
    }
    /// Determine if the layout is in landscrape view or not to emulate.
    pub fn set_landscape(&mut self, is_landscape: bool) {
        self.is_landscape = is_landscape;
    }
    /// Determine if the device is a touch screen or not to emulate.
    pub fn set_touch(&mut self, has_touch: bool) {
        self.has_touch = has_touch;
    }
    /// The scale factor for the screen layout.
    pub fn set_scale_factor(&mut self, device_scale_factor: Option<f64>) {
        self.device_scale_factor = device_scale_factor;
    }
}

#[cfg(feature = "chrome")]
impl From<Viewport> for chromiumoxide::handler::viewport::Viewport {
    fn from(viewport: Viewport) -> Self {
        Self {
            width: viewport.width,
            height: viewport.height,
            device_scale_factor: viewport.device_scale_factor,
            emulating_mobile: viewport.emulating_mobile,
            is_landscape: viewport.is_landscape,
            has_touch: viewport.has_touch,
        }
    }
}

impl From<Viewport> for spider_fingerprint::spoof_viewport::Viewport {
    fn from(viewport: Viewport) -> Self {
        Self {
            width: viewport.width,
            height: viewport.height,
            device_scale_factor: viewport.device_scale_factor,
            emulating_mobile: viewport.emulating_mobile,
            is_landscape: viewport.is_landscape,
            has_touch: viewport.has_touch,
        }
    }
}

#[doc = "Capture page screenshot.\n[captureScreenshot](https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-captureScreenshot)"]
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CaptureScreenshotParams {
    #[doc = "Image compression format (defaults to png)."]
    pub format: Option<CaptureScreenshotFormat>,
    #[doc = "Compression quality from range [0..100] (jpeg only)."]
    pub quality: Option<i64>,
    #[doc = "Capture the screenshot of a given region only."]
    pub clip: Option<ClipViewport>,
    #[doc = "Capture the screenshot from the surface, rather than the view. Defaults to true."]
    pub from_surface: Option<bool>,
    #[doc = "Capture the screenshot beyond the viewport. Defaults to false."]
    pub capture_beyond_viewport: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// The view port clip for screenshots.
pub struct ClipViewport {
    #[doc = "X offset in device independent pixels (dip)."]
    #[cfg_attr(feature = "serde", serde(rename = "x"))]
    pub x: f64,
    #[doc = "Y offset in device independent pixels (dip)."]
    #[cfg_attr(feature = "serde", serde(rename = "y"))]
    pub y: f64,
    #[doc = "Rectangle width in device independent pixels (dip)."]
    #[cfg_attr(feature = "serde", serde(rename = "width"))]
    pub width: f64,
    #[doc = "Rectangle height in device independent pixels (dip)."]
    #[cfg_attr(feature = "serde", serde(rename = "height"))]
    pub height: f64,
    #[doc = "Page scale factor."]
    #[cfg_attr(feature = "serde", serde(rename = "scale"))]
    pub scale: f64,
}

#[cfg(feature = "chrome")]
impl From<ClipViewport> for chromiumoxide::cdp::browser_protocol::page::Viewport {
    fn from(viewport: ClipViewport) -> Self {
        Self {
            x: viewport.x,
            y: viewport.y,
            height: viewport.height,
            width: viewport.width,
            scale: viewport.scale,
        }
    }
}

/// Screenshot configuration.
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScreenShotConfig {
    /// The screenshot params.
    pub params: ScreenshotParams,
    /// Return the bytes of the screenshot on the Page.
    pub bytes: bool,
    /// Store the screenshot to disk. This can be used with output_dir. If disabled will not store the file to the output directory.
    pub save: bool,
    /// The output directory to store the file. Parant folders may be created inside the directory.
    pub output_dir: Option<std::path::PathBuf>,
}

impl ScreenShotConfig {
    /// Create a new screenshot configuration.
    pub fn new(
        params: ScreenshotParams,
        bytes: bool,
        save: bool,
        output_dir: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            params,
            bytes,
            save,
            output_dir,
        }
    }
}

/// The screenshot params for the page.
#[derive(Default, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScreenshotParams {
    /// Chrome DevTools Protocol screenshot options.
    pub cdp_params: CaptureScreenshotParams,
    /// Take full page screenshot.
    pub full_page: Option<bool>,
    /// Make the background transparent (png only).
    pub omit_background: Option<bool>,
}

impl ScreenshotParams {
    /// Create a new ScreenshotParams.
    pub fn new(
        cdp_params: CaptureScreenshotParams,
        full_page: Option<bool>,
        omit_background: Option<bool>,
    ) -> Self {
        Self {
            cdp_params,
            full_page,
            omit_background,
        }
    }
}

#[cfg(feature = "chrome")]
impl From<ScreenshotParams> for chromiumoxide::page::ScreenshotParams {
    fn from(params: ScreenshotParams) -> Self {
        let full_page = if params.full_page.is_some() {
            match params.full_page {
                Some(v) => v,
                _ => false,
            }
        } else {
            std::env::var("SCREENSHOT_FULL_PAGE").unwrap_or_default() == "true"
        };
        let omit_background = if params.omit_background.is_some() {
            match params.omit_background {
                Some(v) => v,
                _ => false,
            }
        } else {
            match std::env::var("SCREENSHOT_OMIT_BACKGROUND") {
                Ok(t) => t == "true",
                _ => true,
            }
        };
        let format = if params.cdp_params.format.is_some() {
            match params.cdp_params.format {
                Some(v) => v.into(),
                _ => chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
            }
        } else {
            chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png
        };

        let params_builder = chromiumoxide::page::ScreenshotParams::builder()
            .format(format)
            .full_page(full_page)
            .omit_background(omit_background);

        let params_builder = if params.cdp_params.quality.is_some() {
            params_builder.quality(params.cdp_params.quality.unwrap_or(75))
        } else {
            params_builder
        };

        let params_builder = if params.cdp_params.clip.is_some() {
            match params.cdp_params.clip {
                Some(vp) => params_builder.clip(
                    chromiumoxide::cdp::browser_protocol::page::Viewport::from(vp),
                ),
                _ => params_builder,
            }
        } else {
            params_builder
        };

        let params_builder = if params.cdp_params.capture_beyond_viewport.is_some() {
            match params.cdp_params.capture_beyond_viewport {
                Some(capture_beyond_viewport) => {
                    params_builder.capture_beyond_viewport(capture_beyond_viewport)
                }
                _ => params_builder,
            }
        } else {
            params_builder
        };

        let params_builder = if params.cdp_params.from_surface.is_some() {
            match params.cdp_params.from_surface {
                Some(from_surface) => params_builder.from_surface(from_surface),
                _ => params_builder,
            }
        } else {
            params_builder
        };

        params_builder.build()
    }
}

#[doc = "The decision on what to do in response to the authorization challenge.  Default means\ndeferring to the default behavior of the net stack, which will likely either the Cancel\nauthentication or display a popup dialog box."]
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AuthChallengeResponseResponse {
    #[default]
    /// The default.
    Default,
    /// Cancel the authentication prompt.
    CancelAuth,
    /// Provide credentials.
    ProvideCredentials,
}

#[doc = "Response to an AuthChallenge.\n[AuthChallengeResponse](https://chromedevtools.github.io/devtools-protocol/tot/Fetch/#type-AuthChallengeResponse)"]
#[derive(Default, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AuthChallengeResponse {
    #[doc = "The decision on what to do in response to the authorization challenge.  Default means\ndeferring to the default behavior of the net stack, which will likely either the Cancel\nauthentication or display a popup dialog box."]
    pub response: AuthChallengeResponseResponse,
    #[doc = "The username to provide, possibly empty. Should only be set if response is\nProvideCredentials."]
    pub username: Option<String>,
    #[doc = "The password to provide, possibly empty. Should only be set if response is\nProvideCredentials."]
    pub password: Option<String>,
}

#[cfg(feature = "chrome")]
impl From<AuthChallengeResponse>
    for chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponse
{
    fn from(auth_challenge_response: AuthChallengeResponse) -> Self {
        Self {
            response: match auth_challenge_response.response {
                AuthChallengeResponseResponse::CancelAuth => chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponseResponse::CancelAuth ,
                AuthChallengeResponseResponse::ProvideCredentials => chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponseResponse::ProvideCredentials ,
                AuthChallengeResponseResponse::Default => chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponseResponse::Default ,

            },
            username: auth_challenge_response.username,
            password: auth_challenge_response.password
        }
    }
}

/// Represents various web automation actions.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WebAutomation {
    /// Runs custom JavaScript code.
    Evaluate(String),
    /// Clicks on an element.
    Click(String),
    /// Clicks on all elements.
    ClickAll(String),
    /// Clicks at the position x and y coordinates.
    ClickPoint {
        /// The horizontal (X) coordinate.
        x: f64,
        /// The vertical (Y) coordinate.
        y: f64,
    },
    /// Click and hold on an element (uses the element clickable point).
    ClickHold {
        /// The selector of the element to click-hold.
        selector: String,
        /// How long to hold (ms).
        hold_ms: u64,
    },
    /// Click and hold at a specific point.
    ClickHoldPoint {
        /// The horizontal (X) coordinate.
        x: f64,
        /// The vertical (Y) coordinate.
        y: f64,
        /// How long to hold (ms).
        hold_ms: u64,
    },
    /// Click-and-drag from one element to another (uses clickable points).
    ClickDrag {
        /// Drag start selector
        from: String,
        /// Drag end selector
        to: String,
        /// Optional modifier (e.g. 8 for Shift). If None, no modifier.
        modifier: Option<i64>,
    },

    /// Click-and-drag from one point to another.
    ClickDragPoint {
        /// Start X
        from_x: f64,
        /// Start Y
        from_y: f64,
        /// End X
        to_x: f64,
        /// End Y
        to_y: f64,
        /// Optional modifier (e.g. 8 for Shift). If None, no modifier.
        modifier: Option<i64>,
    },
    /// Clicks on all elements.
    ClickAllClickable(),
    /// Waits for a fixed duration in milliseconds.
    Wait(u64),
    /// Waits for the next navigation event.
    WaitForNavigation,
    /// Wait for dom updates to stop.
    WaitForDom {
        /// The selector of the element to wait for updates.
        selector: Option<String>,
        ///  The timeout to wait for in ms.
        timeout: u32,
    },
    /// Waits for an element to appear.
    WaitFor(String),
    /// Waits for an element to appear with a timeout.
    WaitForWithTimeout {
        /// The selector of the element to wait for updates.
        selector: String,
        ///  The timeout to wait for in ms.
        timeout: u64,
    },
    /// Waits for an element to appear and then clicks on it.
    WaitForAndClick(String),
    /// Scrolls the screen in the horizontal axis by a specified amount in pixels.
    ScrollX(i32),
    /// Scrolls the screen in the vertical axis by a specified amount in pixels.
    ScrollY(i32),
    /// Fills an input element with a specified value.
    Fill {
        /// The selector of the input element to fill.
        selector: String,
        ///  The value to fill the input element with.
        value: String,
    },
    /// Type with the keyboard.
    Type {
        ///  The value to fill the input element with.
        value: String,
        /// The mofidier to use for the key.
        modifier: Option<i64>,
    },
    /// Scrolls the page until the end.
    InfiniteScroll(u32),
    /// Perform a screenshot on the page - fullscreen and omit background for params.
    Screenshot {
        /// Take a full page screenshot.
        full_page: bool,
        /// Omit the background.
        omit_background: bool,
        /// The output file to store the screenshot.
        output: String,
    },
    /// Only continue to the next automation if the prior step was valid. Use this intermediate after a step to break out of the chain.
    ValidateChain,
}

impl WebAutomation {
    /// Machine-friendly variant name (no params).
    pub fn name(&self) -> &'static str {
        use WebAutomation::*;
        match self {
            Evaluate(_) => "Evaluate",
            Click(_) => "Click",
            ClickDrag { .. } => "ClickDrag",
            ClickDragPoint { .. } => "ClickDragPoint",
            ClickHold { .. } => "ClickHold",
            ClickHoldPoint { .. } => "ClickHoldPoint",
            ClickAll(_) => "ClickAll",
            ClickAllClickable() => "ClickAllClickable",
            ClickPoint { .. } => "ClickPoint",
            Wait(_) => "Wait",
            WaitForNavigation => "WaitForNavigation",
            WaitForDom { .. } => "WaitForDom",
            WaitFor(_) => "WaitFor",
            WaitForWithTimeout { .. } => "WaitForWithTimeout",
            WaitForAndClick(_) => "WaitForAndClick",
            ScrollX(_) => "ScrollX",
            ScrollY(_) => "ScrollY",
            Fill { .. } => "Fill",
            Type { .. } => "Type",
            InfiniteScroll(_) => "InfiniteScroll",
            Screenshot { .. } => "Screenshot",
            ValidateChain => "ValidateChain",
        }
    }

    /// Optional: human-friendly label with key params.
    pub fn label(&self) -> String {
        use WebAutomation::*;
        match self {
            Evaluate(_) => "Evaluate JS".into(),
            Click(s) => format!("Click {}", s),
            ClickHold { selector, hold_ms } => {
                format!("ClickHold {} ({}ms)", selector, hold_ms)
            }
            ClickHoldPoint { x, y, hold_ms } => {
                format!("ClickHoldPoint x:{} y:{} ({}ms)", x, y, hold_ms)
            }
            ClickDrag { from, to, modifier } => {
                format!("ClickDrag {} -> {} modifier={:?}", from, to, modifier)
            }
            ClickDragPoint {
                from_x,
                from_y,
                to_x,
                to_y,
                modifier,
            } => format!(
                "ClickDragPoint ({},{}) -> ({},{}) modifier={:?}",
                from_x, from_y, to_x, to_y, modifier
            ),
            ClickAll(s) => format!("ClickAll {}", s),
            ClickAllClickable() => "ClickAllClickable".into(),
            Wait(ms) => format!("Wait {}ms", ms),
            WaitForNavigation => "WaitForNavigation".into(),
            ClickPoint { x, y } => {
                format!("ClickPoint x:{} y:{}", x, y)
            }
            WaitForDom { selector, timeout } => selector
                .as_ref()
                .map(|s| format!("WaitForDom {} ({}ms)", s, timeout))
                .unwrap_or_else(|| format!("WaitForDom ({}ms)", timeout)),
            WaitFor(s) => format!("WaitFor {}", s),
            WaitForWithTimeout { selector, timeout } => {
                format!("WaitForWithTimeout {} ({}ms)", selector, timeout)
            }
            WaitForAndClick(s) => format!("WaitForAndClick {}", s),
            ScrollX(dx) => format!("ScrollX {}", dx),
            ScrollY(dy) => format!("ScrollY {}", dy),
            Fill { selector, .. } => format!("Fill {}", selector),
            Type { value, modifier } => format!("Type {} modifier={:?}", value, modifier),
            InfiniteScroll(n) => format!("InfiniteScroll {}", n),
            Screenshot {
                full_page,
                omit_background,
                output,
            } => format!(
                "Screenshot full={} omit_bg={} -> {}",
                full_page, omit_background, output
            ),
            ValidateChain => "ValidateChain".into(),
        }
    }
}

impl core::fmt::Display for WebAutomation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.label())
    }
}

#[cfg(feature = "chrome")]
/// Generate the wait for Dom function targeting the element. This defaults to using the body.
pub(crate) fn generate_wait_for_dom_js_v2(
    timeout_ms: u32,
    selector: &str,
    quiet_ms: u32,
    stable_frames: u32,
    require_visible: bool,
    require_images_complete: bool,
) -> String {
    let t = timeout_ms.min(crate::utils::FIVE_MINUTES);
    let q = quiet_ms.max(50).min(t);
    let f = stable_frames.max(1).min(10);
    let s = if selector.is_empty() {
        "body"
    } else {
        selector
    };

    format!(
        r###"(()=>new Promise(R=>{{const S={s:?},T={t},Q={q},F={f},V={vis},I={img},P=["#__next","#__nuxt","#app","#root","main","body"],N=()=>performance.now(),W=e=>{{if(!e)return!1;const t=getComputedStyle(e);if("none"===t.display||"hidden"===t.visibility||"0"===t.opacity)return!1;const r=e.getBoundingClientRect();return r.width>0&&r.height>0}},M=e=>{{if(!e)return!1;const t=e.querySelectorAll("img");for(let e=0;e<t.length;e++){{const r=t[e];if(!r.complete)return!1;if(0===r.naturalWidth&&0===r.naturalHeight&&r.currentSrc)return!1}}return!0}},k=()=>{{let e=document.querySelector(S);if(e)return e;for(let t=0;t<P.length;t++){{if(e=document.querySelector(P[t]))return e}}return null}},s=N();let e=null,t=null,r=s,o=0;const a=n=>{{t&&t.disconnect(),t=new MutationObserver(()=>{{r=N(),o=0}}),t.observe(n,{{subtree:!0,childList:!0,attributes:!0,characterData:!0}})}},i=()=>{{const n=N();if(n-s>=T)return t&&t.disconnect(),void R(!1);(!e||!document.contains(e))&&(e=k())&&((r=n,o=0),a(e));e&&(V&&!W(e)?o=0:I&&!M(e)?o=0:n-r>=Q?(o++,o>=F&&(t&&t.disconnect(),R(!0))):o=0),requestAnimationFrame(i)}};i()}}))()"###,
        t = t,
        q = q,
        f = f,
        vis = if require_visible { "true" } else { "false" },
        img = if require_images_complete {
            "true"
        } else {
            "false"
        },
    )
}

#[cfg(feature = "chrome")]
/// Generate the wait for Dom function targeting the element. This defaults to using the body.
pub(crate) fn generate_wait_for_dom_js_code_with_selector(
    timeout: u32,
    selector: Option<&str>,
) -> String {
    let t = timeout.min(crate::utils::FIVE_MINUTES);
    let s = selector.unwrap_or("body");
    format!(
        "new Promise((r,j)=>{{const s='{s}',t={t},i=220,n=50;let l=Date.now(),el,o,d,c;const check=()=>{{el=document.querySelector(s);if(!el)return;clearInterval(wait);l=Date.now();o=new MutationObserver(()=>{{l=Date.now();}});o.observe(el,{{childList:!0,subtree:!0,attributes:!0,characterData:!0}});d=setTimeout(()=>{{clearInterval(c),o.disconnect(),j(new Error('Dom Timeout.'))}},t);c=setInterval(()=>{{Date.now()-l>=i&&(clearTimeout(d),clearInterval(c),o.disconnect(),r(!0))}},n);}};const wait=setInterval(check,n);check();}});",
        s = s,
        t = t
    )
}

// #[cfg(feature = "chrome")]
// /// Generate the wait for Dom function targeting the element. This defaults to using the body.
// pub(crate) fn generate_wait_for_dom_js_code_with_selector_base(
//     timeout: u32,
//     selector: &str,
// ) -> String {
//     generate_wait_for_dom_js_code_with_selector(
//         timeout,
//         if selector.is_empty() {
//             None
//         } else {
//             Some(selector)
//         },
//     )
// }

#[cfg(feature = "chrome")]
const CLICKABLE_SELECTOR: &str = concat!(
    "button:not([disabled]),",
    "input[type='button']:not([disabled]),",
    "input[type='submit']:not([disabled]),",
    "input[type='reset']:not([disabled]),",
    "input[type='checkbox']:not([disabled]),",
    "input[type='radio']:not([disabled]),",
    "label[for],",
    "summary,",
    "select:not([disabled]),",
    "textarea:not([disabled]),",
    "[role='button']:not([aria-disabled='true']),",
    "[onclick],",
    "[contenteditable=''],[contenteditable='true'],",
    // focusables as a last resort
    "[tabindex]:not([tabindex^='-'],a[href])"
);

impl WebAutomation {
    #[cfg(feature = "chrome")]
    /// Run the web automation step.
    pub async fn run(&self, page: &chromiumoxide::Page) -> bool {
        use crate::utils::wait_for_selector;
        use std::time::Duration;

        let mut valid = false;

        match self {
            WebAutomation::Evaluate(js) => {
                valid = page.evaluate(js.as_str()).await.is_ok();
            }
            WebAutomation::Click(selector) => {
                if let Ok(ele) = page.find_element(selector).await {
                    valid = ele.click().await.is_ok();
                }
            }

            WebAutomation::WaitForWithTimeout { selector, timeout } => {
                valid =
                    wait_for_selector(page, Some(Duration::from_millis(*timeout)), &selector).await;
            }
            WebAutomation::Wait(ms) => {
                tokio::time::sleep(Duration::from_millis(*ms)).await;
                valid = true;
            }
            WebAutomation::ClickHold { selector, hold_ms } => {
                if let Ok(ele) = page.find_element(selector).await {
                    if let Ok(pt) = ele.clickable_point().await {
                        valid = page
                            .click_and_hold(pt, Duration::from_millis(*hold_ms))
                            .await
                            .is_ok();
                    }
                }
            }
            WebAutomation::ClickHoldPoint { x, y, hold_ms } => {
                let pt = chromiumoxide::layout::Point { x: *x, y: *y };
                valid = page
                    .click_and_hold(pt, Duration::from_millis(*hold_ms))
                    .await
                    .is_ok();
            }
            WebAutomation::ClickAll(selector) => {
                if let Ok(eles) = page.find_elements(selector).await {
                    for ele in eles {
                        valid = ele.click().await.is_ok();
                    }
                }
            }
            WebAutomation::ClickDrag { from, to, modifier } => {
                if let (Ok(from_el), Ok(to_el)) =
                    (page.find_element(from).await, page.find_element(to).await)
                {
                    if let (Ok(p1), Ok(p2)) = (
                        from_el.clickable_point().await,
                        to_el.clickable_point().await,
                    ) {
                        valid = match modifier {
                            Some(m) => page.click_and_drag_with_modifier(p1, p2, *m).await.is_ok(),
                            None => page.click_and_drag(p1, p2).await.is_ok(),
                        };
                    }
                }
            }

            WebAutomation::ClickDragPoint {
                from_x,
                from_y,
                to_x,
                to_y,
                modifier,
            } => {
                let p1 = chromiumoxide::layout::Point {
                    x: *from_x,
                    y: *from_y,
                };
                let p2 = chromiumoxide::layout::Point { x: *to_x, y: *to_y };

                valid = match modifier {
                    Some(m) => page.click_and_drag_with_modifier(p1, p2, *m).await.is_ok(),
                    None => page.click_and_drag(p1, p2).await.is_ok(),
                };
            }
            WebAutomation::ClickAllClickable() => {
                if let Ok(eles) = page.find_elements(CLICKABLE_SELECTOR).await {
                    for ele in eles {
                        valid = ele.click().await.is_ok();
                    }
                }
            }
            WebAutomation::ClickPoint { x, y } => {
                valid = page
                    .click(chromiumoxide::layout::Point { x: *x, y: *y })
                    .await
                    .is_ok();
            }
            WebAutomation::WaitForDom { selector, timeout } => {
                valid = page
                    .evaluate(
                        generate_wait_for_dom_js_code_with_selector(*timeout, selector.as_deref())
                            .as_str(),
                    )
                    .await
                    .is_ok();
            }
            WebAutomation::WaitFor(selector) => {
                valid = wait_for_selector(page, Some(Duration::from_secs(60)), &selector).await;
            }
            WebAutomation::WaitForNavigation => {
                valid = page.wait_for_navigation().await.is_ok();
            }
            WebAutomation::WaitForAndClick(selector) => {
                valid = wait_for_selector(page, Some(Duration::from_secs(60)), &selector).await;
                if let Ok(ele) = page.find_element(selector).await {
                    valid = ele.click().await.is_ok();
                }
            }
            WebAutomation::ScrollX(px) => {
                valid = page
                    .scroll_by((*px as f32).into(), 0.0, Default::default())
                    .await
                    .is_ok()
            }
            WebAutomation::ScrollY(px) => {
                valid = page
                    .scroll_by(0.0, (*px as f32).into(), Default::default())
                    .await
                    .is_ok()
            }
            WebAutomation::Fill { selector, value } => {
                if let Ok(ele) = page.find_element(selector).await {
                    if let Ok(el) = ele.click().await {
                        valid = el.type_str(value).await.is_ok();
                    }
                }
            }
            WebAutomation::Type { value, modifier } => {
                valid = page.type_str_with_modifier(value, *modifier).await.is_ok()
            }
            WebAutomation::InfiniteScroll(duration) => {
                valid = page.evaluate(set_dynamic_scroll(*duration)).await.is_ok();
            }
            WebAutomation::Screenshot {
                full_page,
                omit_background,
                output,
            } => {
                let mut cdp_params: CaptureScreenshotParams = CaptureScreenshotParams::default();
                cdp_params.format = Some(CaptureScreenshotFormat::Png);

                let screenshot_params =
                    ScreenshotParams::new(cdp_params, Some(*full_page), Some(*omit_background));

                valid = page
                    .save_screenshot(screenshot_params, output)
                    .await
                    .is_ok();
            }
            _ => (),
        };

        valid
    }
}

/// Set a dynamic time to scroll.
pub fn set_dynamic_scroll(timeout: u32) -> String {
    let timeout = timeout.min(crate::utils::FIVE_MINUTES);
    let s = string_concat!(
        r###"document.addEventListener('DOMContentLoaded',e=>{let t=null,o=null,n="###,
        timeout.to_string(),
        r###",a=Date.now(),i=Date.now(),r=()=>{window.scrollTo(0,document.body.scrollHeight)},l=()=>{o&&o.disconnect(),console.log('Stopped checking for new content.')},c=(e,n)=>{e.forEach(e=>{if(e.isIntersecting){i=Date.now();const n=Date.now();if(n-a>=t||n-i>=1e4)return void l();r(),t=document.querySelector('body > *:last-child'),o.observe(t)}})},s=()=>{t&&(o=new IntersectionObserver(c),o.observe(t))},d=()=>{['load','error','abort'].forEach(e=>{window.addEventListener(e,()=>{const e=document.querySelector('body > *:last-child');e!==t&&(i=Date.now(),t=e,o.observe(t))})})},u=()=>{r(),t=document.querySelector('body > *:last-child'),s(),d()};u(),setTimeout(l,n)});"###
    );

    s
}

/// Execution scripts to run on the page when using chrome by url.
pub type ExecutionScriptsMap = hashbrown::HashMap<String, String>;
/// Automation scripts to run on the page when using chrome by url.
pub type AutomationScriptsMap = hashbrown::HashMap<String, Vec<WebAutomation>>;

/// Execution scripts to run on the page when using chrome by url.
pub type ExecutionScripts = Trie<String>;
/// Automation scripts to run on the page when using chrome by url.
pub type AutomationScripts = Trie<Vec<WebAutomation>>;

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Chrome request interception configurations.
pub struct RequestInterceptConfiguration {
    /// Request interception enabled?
    pub enabled: bool,
    /// Block visuals. By default this is enabled. This will prevent Prefetch, Ping, and some javascript from rendering.
    pub block_visuals: bool,
    /// Block stylesheets.
    pub block_stylesheets: bool,
    /// Block javascript only allowing critcal framework or lib based javascript to render..
    pub block_javascript: bool,
    /// Block analytics.
    pub block_analytics: bool,
    /// Block ads. Requires the `adblock` feature flag.
    pub block_ads: bool,
    /// Intercept Manager
    pub intercept_manager: NetworkInterceptManager,
    /// Whitelist patterns.
    pub whitelist_patterns: Option<Vec<String>>,
}

impl RequestInterceptConfiguration {
    /// Setup a new intercept config
    pub fn new(enabled: bool) -> RequestInterceptConfiguration {
        RequestInterceptConfiguration {
            enabled,
            block_javascript: false,
            block_visuals: true,
            block_analytics: true,
            block_stylesheets: true,
            block_ads: true,
            ..Default::default()
        }
    }
    /// Setup a new intercept config with a custom intercept manager.
    pub fn new_manager(
        enabled: bool,
        url: &Option<Box<url::Url>>,
    ) -> RequestInterceptConfiguration {
        RequestInterceptConfiguration {
            enabled,
            block_javascript: false,
            block_visuals: true,
            block_analytics: true,
            block_stylesheets: true,
            intercept_manager: NetworkInterceptManager::new(url),
            ..Default::default()
        }
    }

    /// Setup the network request manager type.
    pub fn setup_intercept_manager(&mut self, url: &Option<Box<url::Url>>) {
        self.intercept_manager = NetworkInterceptManager::new(url);
    }

    /// Set the whitelist patterns.
    pub fn set_whitelist_patterns(&mut self, whitelist_patterns: Option<Vec<String>>) {
        self.whitelist_patterns = whitelist_patterns;
    }

    /// Block all request besides html and the important stuff.
    pub fn block_all(&mut self) -> &Self {
        self.block_javascript = true;
        self.block_analytics = true;
        self.block_stylesheets = true;
        self.block_visuals = true;
        self.block_ads = true;
        self
    }
}

/// Convert ExecutionScripts to Trie.
pub fn convert_to_trie_execution_scripts(
    input: &Option<ExecutionScriptsMap>,
) -> Option<Trie<String>> {
    match input {
        Some(ref scripts) => {
            let mut trie = Trie::new();
            for (path, script) in scripts {
                trie.insert(path, script.clone());
            }
            Some(trie)
        }
        None => None,
    }
}

/// Convert AutomationScripts to Trie.
pub fn convert_to_trie_automation_scripts(
    input: &Option<AutomationScriptsMap>,
) -> Option<Trie<Vec<WebAutomation>>> {
    match input {
        Some(ref scripts) => {
            let mut trie = Trie::new();
            for (path, script_list) in scripts {
                trie.insert(path, script_list.clone());
            }
            Some(trie)
        }
        None => None,
    }
}

/// Eval execution scripts.
#[cfg(feature = "chrome")]
pub async fn eval_execution_scripts(
    page: &chromiumoxide::Page,
    target_url: &str,
    execution_scripts: &Option<ExecutionScripts>,
) {
    if let Some(scripts) = &execution_scripts {
        if let Some(script) = scripts.search(target_url) {
            let _ = page.evaluate(script.as_str()).await;
        } else if scripts.match_all {
            if let Some(script) = scripts.root.value.as_ref() {
                let _ = page.evaluate(script.as_str()).await;
            }
        }
    }
}

/// Run automation scripts.
#[cfg(feature = "chrome")]
pub async fn eval_automation_scripts(
    page: &chromiumoxide::Page,
    target_url: &str,
    automation_scripts: &Option<AutomationScripts>,
) {
    if let Some(script_map) = automation_scripts {
        if let Some(scripts) = script_map.search(target_url) {
            let mut valid = false;

            for script in scripts {
                if script == &WebAutomation::ValidateChain && !valid {
                    break;
                }
                match tokio::time::timeout(tokio::time::Duration::from_secs(60), script.run(page))
                    .await
                {
                    Ok(next) => valid = next,
                    Err(elasped) => {
                        log::warn!("Script execution timed out for: {target_url} - {elasped}")
                    }
                }
            }
        } else if script_map.match_all {
            if let Some(scripts) = script_map.root.value.as_ref() {
                let mut valid = false;

                for script in scripts {
                    if script == &WebAutomation::ValidateChain && !valid {
                        break;
                    }
                    match tokio::time::timeout(
                        tokio::time::Duration::from_secs(60),
                        script.run(page),
                    )
                    .await
                    {
                        Ok(next) => valid = next,
                        Err(elasped) => {
                            log::warn!("Script execution timed out for: {target_url} - {elasped}")
                        }
                    }
                }
            }
        }
    }
}

#[cfg(feature = "chrome")]
/// default observer
pub const OBSERVER_SYMBOL_KEY: &str = "obs:3f2d6c07-5c01-4e2e-9ab0-8a8b9a7b2f4c";
/// Installs a MutationObserver + URL change hooks and a drain function.
#[cfg(feature = "chrome")]
pub const INSTALL_OBSERVER_JS: &str = r##"(function(){const K=Symbol.for("obs:3f2d6c07-5c01-4e2e-9ab0-8a8b9a7b2f4c");if(globalThis[K])return globalThis[K];const Q=[],cap=100,cssPath=e=>{if(!e||1!==e.nodeType)return "#text";const t=[];for(;e&&1===e.nodeType&&e!==document.documentElement;){let n=e.nodeName.toLowerCase();if(e.id){n+="#"+e.id,t.unshift(n);break}let r=e,o=1;for(;(r=r.previousElementSibling)!=null;)if(r.nodeName===e.nodeName)o++;t.unshift(n+":nth-of-type("+o+")"),e=e.parentElement}return t.length?t.join(">"):"html"},state=()=>({url:location.href,title:document.title,viewport:{w:innerWidth,h:innerHeight,dpr:devicePixelRatio||1},scroll:{x:pageXOffset||0,y:pageYOffset||0}}),serialize=m=>"childList"===m.type?{type:"childList",target:cssPath(m.target),added:Array.from(m.addedNodes||[]).slice(0,3).map(n=>1===n.nodeType?n.outerHTML:"#text"),removed:Array.from(m.removedNodes||[]).slice(0,3).map(n=>1===n.nodeType?n.outerHTML:"#text" )}:"attributes"===m.type?{type:"attributes",target:cssPath(m.target),attribute:m.attributeName||"",old:m.oldValue??null,new:m.attributeName?m.target.getAttribute(m.attributeName):null}:{type:m.type,target:cssPath(m.target)},push=e=>{Q.push({t:Date.now(),state:state(),mutations:e?e.map(serialize):[]}),Q.length>cap&&Q.splice(0,Q.length-cap)};new MutationObserver(l=>push(Array.from(l))).observe(document.documentElement,{subtree:!0,childList:!0,attributes:!0,characterData:!1,attributeOldValue:!0});const notify=()=>push(null);["pushState","replaceState"].forEach(k=>{const o=history[k];history[k]=function(){const r=o.apply(this,arguments);return dispatchEvent(new Event("locationchange")),r}});addEventListener("popstate",()=>dispatchEvent(new Event("locationchange")));addEventListener("hashchange",()=>dispatchEvent(new Event("locationchange")));addEventListener("locationchange",notify,{passive:!0});const emitVal=el=>{const now=el&&null!=el.value?""+el.value:null,old=Object.prototype.hasOwnProperty.call(el,"__oldVal")?el.__oldVal:null;el.__oldVal=now,Q.push({t:Date.now(),state:state(),action:"input",target:cssPath(el),old:old,new:now}),Q.length>cap&&Q.splice(0,Q.length-cap)};addEventListener("input",e=>{const t=e.target;if(t&&("INPUT"===t.tagName||"TEXTAREA"===t.tagName))emitVal(t)},{capture:!0,passive:!0});addEventListener("change",e=>{const t=e.target;if(t&&("INPUT"===t.tagName||"TEXTAREA"===t.tagName))emitVal(t)},{capture:!0});addEventListener("click",e=>{const el=e.target instanceof Element?e.target:null;Q.push({t:Date.now(),state:state(),action:"click",target:el?cssPath(el):null,meta:el?{tag:el.tagName,id:el.id||null,class:el.className||null,href:el.getAttribute("href")||null,text:(el.textContent||"").trim().slice(0,120)}:null}),Q.length>cap&&Q.splice(0,Q.length-cap)},{capture:!0});const handle={drain:function(){const out=Q.slice();return Q.length=0,out}};return globalThis[K]=handle})()"##;

/// Drains the recorded changes as an array; returns [] if not installed.
#[cfg(feature = "chrome")]
pub const DRAIN_CHANGES_JS: &str = r##"(function(){const h=globalThis[Symbol.for("obs:3f2d6c07-5c01-4e2e-9ab0-8a8b9a7b2f4c")];return h&&h.drain?h.drain():[]})()"##;

#[cfg(feature = "chrome")]
/// Generate a random Symbol key for this session/run.
fn gen_symbol_key() -> String {
    const ALPH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut s = String::with_capacity(24);
    for _ in 0..24 {
        let i = fastrand::usize(..ALPH.len());
        s.push(ALPH[i] as char);
    }
    format!("obs:{}", s)
}

#[cfg(feature = "chrome")]
/// Run automation scripts with state tracking.
pub async fn eval_automation_scripts_tracking(
    page: &chromiumoxide::Page,
    target_url: &str,
    automation_scripts: &Option<AutomationScripts>,
    automation_data: &mut Vec<crate::page::AutomationResults>,
) {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;

    fn eval_expr(expr: &str) -> EvaluateParams {
        EvaluateParams::builder()
            .expression(expr)
            .eval_as_function_fallback(true)
            .build()
            .expect("eval_expr build failed")
    }

    fn eval_value(expr: &str) -> EvaluateParams {
        EvaluateParams::builder()
            .expression(expr)
            .return_by_value(true)
            .build()
            .expect("eval_value build failed")
    }

    let symbol = gen_symbol_key();

    let install = INSTALL_OBSERVER_JS.replacen(OBSERVER_SYMBOL_KEY, &symbol, 1);
    let drain = DRAIN_CHANGES_JS.replacen(OBSERVER_SYMBOL_KEY, &symbol, 1);

    let _ = page.evaluate(eval_expr(&install)).await;
    let _ = page.evaluate(eval_value(&drain)).await;

    /// The generic screenshot params.
    fn ss_params() -> chromiumoxide::page::ScreenshotParams {
        chromiumoxide::page::ScreenshotParams::builder()
            .format(chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png)
            .full_page(true)
            .quality(45)
            .omit_background(false)
            .build()
    }

    // Helper to drain recorded changes as a single JSON string
    async fn drain_changes_json(
        page: &chromiumoxide::Page,
        drain_eval: &str,
    ) -> Option<serde_json::Value> {
        if let Ok(eval) = page.evaluate(eval_value(drain_eval)).await {
            if let Some(value) = eval.value() {
                Some(value.clone())
            } else {
                None
            }
        } else {
            None
        }
    }

    let mut run_scripts: Option<&Vec<WebAutomation>> = None;

    if let Some(script_map) = automation_scripts {
        if let Some(scripts) = script_map.search(target_url) {
            run_scripts = Some(scripts);
        } else if script_map.match_all {
            if let Some(scripts) = script_map.root.value.as_ref() {
                run_scripts = Some(scripts);
            }
        }
    }

    if let Some(scripts) = run_scripts {
        let mut valid = false;

        for (idx, script) in scripts.iter().enumerate() {
            if script == &WebAutomation::ValidateChain && !valid {
                break;
            }

            let step_name = format!("step_{}_{}", idx + 1, script.label());

            let step_result =
                tokio::time::timeout(tokio::time::Duration::from_secs(60), script.run(page)).await;

            let mut automation_results = crate::page::AutomationResults::default();
            automation_results.input = format!("automation:{} url:{}", step_name, target_url);

            match step_result {
                Ok(next) => {
                    valid = next;
                    if let Some(changes_json) = drain_changes_json(page, &drain).await {
                        automation_results.content_output = changes_json;
                    }
                    if let Ok(ss) = page.screenshot(ss_params()).await {
                        automation_results.screenshot_output = Some(STANDARD.encode(&ss));
                    }
                }
                Err(elapsed) => {
                    automation_results.error = Some(format!(
                        "Script execution timed out for: {} - {}",
                        target_url, elapsed
                    ));
                    if let Some(changes_json) = drain_changes_json(page, &drain).await {
                        automation_results.content_output = changes_json;
                    }
                    if let Ok(ss) = page.screenshot(ss_params()).await {
                        automation_results.screenshot_output = Some(STANDARD.encode(&ss));
                    }
                    log::warn!("Script execution timed out for: {target_url} - {elapsed}");
                }
            }

            automation_data.push(automation_results);
        }
    }
}
