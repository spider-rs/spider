use crate::tokio_stream::StreamExt;
use crate::utils::log;
use chromiumoxide_fork::{Browser, BrowserConfig};
use tokio::task;

/// get chrome configuration
#[cfg(not(feature = "chrome_headed"))]
pub fn get_browser_config(
    proxies: &Option<Box<Vec<string_concat::String>>>,
) -> Option<BrowserConfig> {
    use std::time::Duration;
    let builder = BrowserConfig::builder()
        .disable_default_args()
        .request_timeout(Duration::from_secs(30));

    let builder = match proxies {
        Some(proxies) => {
            let mut chrome_args = Vec::from(CHROME_ARGS.map(|e| e.replace("://", "=").to_string()));

            chrome_args.push(string_concat!(
                r#"--proxy-server=""#,
                proxies.join(";"),
                r#"""#
            ));

            builder.args(chrome_args)
        }
        _ => builder.args(CHROME_ARGS),
    };
    let builder = if std::env::var("CHROME_BIN").is_ok() {
        match std::env::var("CHROME_BIN") {
            Ok(v) => builder.chrome_executable(v),
            _ => builder,
        }
    } else {
        builder
    };
    match builder.build() {
        Ok(b) => Some(b),
        Err(error) => {
            log("", error);
            None
        }
    }
}

/// get chrome configuration headful
#[cfg(feature = "chrome_headed")]
pub fn get_browser_config(
    proxies: &Option<Box<Vec<string_concat::String>>>,
) -> Option<BrowserConfig> {
    use std::time::Duration;
    let builder = BrowserConfig::builder()
        .disable_default_args()
        .request_timeout(Duration::from_secs(30))
        .no_sandbox()
        .with_head();

    let builder = match proxies {
        Some(proxies) => {
            let mut chrome_args = Vec::from(CHROME_ARGS.map(|e| e.replace("://", "=").to_string()));

            chrome_args.push(string_concat!(
                r#"--proxy-server=""#,
                proxies.join(";"),
                r#"""#
            ));

            builder.args(chrome_args)
        }
        _ => builder.args(CHROME_ARGS),
    };
    let builder = if std::env::var("CHROME_BIN").is_ok() {
        match std::env::var("CHROME_BIN") {
            Ok(v) => builder.chrome_executable(v),
            _ => builder,
        }
    } else {
        builder
    };
    match builder.build() {
        Ok(b) => Some(b),
        Err(error) => {
            log("", error);
            None
        }
    }
}

/// launch a chromium browser and wait until the instance is up.
pub async fn launch_browser(
    proxies: &Option<Box<Vec<string_concat::String>>>,
) -> Option<(Browser, tokio::task::JoinHandle<()>)> {
    let b_conf = match std::env::var("CHROME_URL") {
        Ok(v) => match Browser::connect(&v).await {
            Ok(browser) => Some(browser),
            _ => None,
        },
        _ => match get_browser_config(&proxies) {
            Some(browser_config) => match Browser::launch(browser_config).await {
                Ok(browser) => Some(browser),
                _ => None,
            },
            _ => None,
        },
    };

    match b_conf {
        Some(c) => {
            let (browser, mut handler) = c;
            // spawn a new task that continuously polls the handler
            let handle = task::spawn(async move {
                while let Some(h) = handler.next().await {
                    if h.is_err() {
                        break;
                    }
                }
            });

            Some((browser, handle))
        }
        _ => None,
    }
}

#[cfg(not(feature = "chrome_cpu"))]
/// static chrome arguments to start application ref [https://github.com/a11ywatch/chrome/blob/main/src/main.rs#L13]
static CHROME_ARGS: [&'static str; 59] = [
    "--headless",
    "--no-sandbox",
    "--no-first-run",
    "--hide-scrollbars",
    // "--allow-pre-commit-input",
    // "--user-data-dir=~/.config/google-chrome",
    "--allow-running-insecure-content",
    "--autoplay-policy=user-gesture-required",
    "--ignore-certificate-errors",
    "--no-default-browser-check",
    "--no-zygote",
    "--disable-setuid-sandbox",
    "--disable-dev-shm-usage", // required or else docker containers may crash not enough memory
    "--disable-threaded-scrolling",
    "--disable-demo-mode",
    "--disable-dinosaur-easter-egg",
    "--disable-fetching-hints-at-navigation-start",
    "--disable-site-isolation-trials",
    "--disable-web-security",
    "--disable-threaded-animation",
    "--disable-sync",
    "--disable-print-preview",
    "--disable-partial-raster",
    "--disable-in-process-stack-traces",
    "--disable-v8-idle-tasks",
    "--disable-low-res-tiling",
    "--disable-speech-api",
    "--disable-smooth-scrolling",
    "--disable-default-apps",
    "--disable-prompt-on-repost",
    "--disable-domain-reliability",
    "--disable-component-update",
    "--disable-background-timer-throttling",
    "--disable-breakpad",
    "--disable-software-rasterizer",
    "--disable-extensions",
    "--disable-popup-blocking",
    "--disable-hang-monitor",
    "--disable-image-animation-resync",
    "--disable-client-side-phishing-detection",
    "--disable-component-extensions-with-background-pages",
    "--disable-ipc-flooding-protection",
    "--disable-background-networking",
    "--disable-renderer-backgrounding",
    "--disable-field-trial-config",
    "--disable-back-forward-cache",
    "--disable-backgrounding-occluded-windows",
    // "--enable-automation",
    "--log-level=3",
    "--enable-logging=stderr",
    "--enable-features=SharedArrayBuffer,NetworkService,NetworkServiceInProcess",
    "--metrics-recording-only",
    "--use-mock-keychain",
    "--force-color-profile=srgb",
    "--mute-audio",
    "--no-service-autorun",
    "--password-store=basic",
    "--export-tagged-pdf",
    "--no-pings",
    "--use-gl=swiftshader",
    "--window-size=1920,1080",
    "--disable-features=AudioServiceOutOfProcess,IsolateOrigins,site-per-process,ImprovedCookieControls,LazyFrameLoading,GlobalMediaControls,DestroyProfileOnBrowserClose,MediaRouter,DialMediaRouteProvider,AcceptCHFrame,AutoExpandDetailsElement,CertificateTransparencyComponentUpdater,AvoidUnnecessaryBeforeUnloadCheckSync,Translate"
];

#[cfg(feature = "chrome_cpu")]
/// static chrome arguments to start application ref [https://github.com/a11ywatch/chrome/blob/main/src/main.rs#L13]
static CHROME_ARGS: [&'static str; 62] = [
    "--headless",
    "--no-sandbox",
    "--no-first-run",
    "--hide-scrollbars",
    // "--allow-pre-commit-input",
    // "--user-data-dir=~/.config/google-chrome",
    "--allow-running-insecure-content",
    "--autoplay-policy=user-gesture-required",
    "--ignore-certificate-errors",
    "--no-default-browser-check",
    "--no-zygote",
    "--in-process-gpu",
    "--disable-gpu",
    "--disable-gpu-sandbox",
    "--disable-setuid-sandbox",
    "--disable-dev-shm-usage", // required or else docker containers may crash not enough memory
    "--disable-threaded-scrolling",
    "--disable-demo-mode",
    "--disable-dinosaur-easter-egg",
    "--disable-fetching-hints-at-navigation-start",
    "--disable-site-isolation-trials",
    "--disable-web-security",
    "--disable-threaded-animation",
    "--disable-sync",
    "--disable-print-preview",
    "--disable-partial-raster",
    "--disable-in-process-stack-traces",
    "--disable-v8-idle-tasks",
    "--disable-low-res-tiling",
    "--disable-speech-api",
    "--disable-smooth-scrolling",
    "--disable-default-apps",
    "--disable-prompt-on-repost",
    "--disable-domain-reliability",
    "--disable-component-update",
    "--disable-background-timer-throttling",
    "--disable-breakpad",
    "--disable-software-rasterizer",
    "--disable-extensions",
    "--disable-popup-blocking",
    "--disable-hang-monitor",
    "--disable-image-animation-resync",
    "--disable-client-side-phishing-detection",
    "--disable-component-extensions-with-background-pages",
    "--disable-ipc-flooding-protection",
    "--disable-background-networking",
    "--disable-renderer-backgrounding",
    "--disable-field-trial-config",
    "--disable-back-forward-cache",
    "--disable-backgrounding-occluded-windows",
    // "--enable-automation",
    "--log-level=3",
    "--enable-logging=stderr",
    "--enable-features=SharedArrayBuffer,NetworkService,NetworkServiceInProcess",
    "--metrics-recording-only",
    "--use-mock-keychain",
    "--force-color-profile=srgb",
    "--mute-audio",
    "--no-service-autorun",
    "--password-store=basic",
    "--export-tagged-pdf",
    "--no-pings",
    "--use-gl=swiftshader",
    "--window-size=1920,1080",
    "--disable-features=AudioServiceOutOfProcess,IsolateOrigins,site-per-process,ImprovedCookieControls,LazyFrameLoading,GlobalMediaControls,DestroyProfileOnBrowserClose,MediaRouter,DialMediaRouteProvider,AcceptCHFrame,AutoExpandDetailsElement,CertificateTransparencyComponentUpdater,AvoidUnnecessaryBeforeUnloadCheckSync,Translate"
];
