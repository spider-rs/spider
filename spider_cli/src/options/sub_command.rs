use clap::{Args, Subcommand};

#[derive(Args, Debug, Clone)]
pub struct AuthenticatedPageArgs {
    /// Target page URL. Falls back to the top-level --url if omitted.
    #[clap(short, long)]
    pub url: Option<String>,
    /// Prepare or reuse the local browser profile for manual login, then exit.
    #[clap(long)]
    pub prepare_profile: bool,
    /// Directory for crawl artifacts.
    #[clap(long, default_value = "authenticated_page_output")]
    pub output_dir: String,
    /// HTML output file name or absolute path.
    #[clap(long, default_value = "page.html")]
    pub output_html: String,
    /// Extracted JSON output file name or absolute path.
    #[clap(long, default_value = "page_extracted.json")]
    pub output_json: String,
    /// Skip downloading images into the output directory.
    #[clap(long)]
    pub no_download_images: bool,
    /// Title selectors used for extraction.
    #[clap(long)]
    pub title_selectors: Option<String>,
    /// Content selectors used for extraction.
    #[clap(long)]
    pub content_selectors: Option<String>,
    /// Image selectors used for extraction.
    #[clap(long, default_value = "img")]
    pub image_selectors: String,
    /// Browser user-data-dir to reuse for authenticated browsing.
    #[clap(long)]
    pub chrome_user_data_dir: Option<String>,
    /// Chrome profile directory inside the user-data-dir.
    #[clap(long, default_value = "Default")]
    pub chrome_profile_dir: String,
    /// Chrome executable path.
    #[clap(long)]
    pub chrome_bin: Option<String>,
    /// Existing CDP endpoint like http://127.0.0.1:9222/json/version.
    #[clap(long)]
    pub chrome_connection_url: Option<String>,
    /// Local debugging port for the dedicated browser.
    #[clap(long, default_value_t = 9222)]
    pub chrome_debugging_port: u16,
    /// Page to open when preparing or launching the dedicated browser.
    #[clap(long)]
    pub chrome_start_url: Option<String>,
    /// Launch the dedicated browser in headless mode.
    #[clap(long)]
    pub chrome_headless: bool,
    /// Extra Chromium arguments passed as a single shell-style string.
    #[clap(long)]
    pub chrome_extra_args: Option<String>,
    /// Explicit proxy for the dedicated browser and image downloads.
    #[clap(long)]
    pub chrome_proxy: Option<String>,
    /// Inherit terminal proxy env vars instead of clearing them.
    #[clap(long)]
    pub chrome_inherit_proxy: bool,
    /// Optional cookie header fallback.
    #[clap(long)]
    pub cookie: Option<String>,
    /// Override the browser-like user agent header.
    #[clap(long)]
    pub user_agent: Option<String>,
    /// Override Accept-Language.
    #[clap(long)]
    pub accept_language_header: Option<String>,
    /// Optional Referer header.
    #[clap(long)]
    pub referer_url: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Crawl the website extracting links.
    CRAWL {
        /// sequentially one by one crawl pages
        #[clap(short, long)]
        sync: bool,
        /// stdout all links crawled
        #[clap(short, long)]
        output_links: bool,
    },
    /// Scrape the website extracting html and links returning the output as jsonl.
    SCRAPE {
        /// stdout all pages links crawled
        #[clap(short, long)]
        output_links: bool,
        /// stdout all pages html crawled
        #[clap(long)]
        output_html: bool,
    },
    /// Download html markup to destination.
    DOWNLOAD {
        /// store files at target destination
        #[clap(short, long)]
        target_destination: Option<String>,
    },
    /// Capture a single authenticated page with a reusable local browser profile.
    AuthenticatedPage(AuthenticatedPageArgs),
}
