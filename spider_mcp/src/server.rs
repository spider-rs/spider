use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use std::sync::Arc;

use crate::state::SharedState;
use crate::tools::{
    crawl::CrawlParams, links::LinksParams, scrape::ScrapeParams, transform::TransformParams,
};

#[derive(Clone)]
pub struct SpiderMcpServer {
    state: Arc<SharedState>,
    #[allow(dead_code)]
    tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
}

impl SpiderMcpServer {
    pub fn new() -> Self {
        let state = Arc::new(SharedState::new());
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl SpiderMcpServer {
    #[tool(
        name = "spider_scrape",
        description = "Fetch a web page and return its content as markdown, text, HTML, or XML. Supports Chrome rendering for JavaScript-heavy sites."
    )]
    async fn scrape(&self, Parameters(params): Parameters<ScrapeParams>) -> Result<String, String> {
        crate::tools::scrape::run(params).await
    }

    #[tool(
        name = "spider_crawl",
        description = "Crawl a website discovering linked pages up to a configurable depth and limit. Returns page content in the requested format."
    )]
    async fn crawl(&self, Parameters(params): Parameters<CrawlParams>) -> Result<String, String> {
        crate::tools::crawl::run(params, self.state.clone()).await
    }

    #[tool(
        name = "spider_links",
        description = "Extract all links from a web page without fetching full content. Lightweight alternative to crawling."
    )]
    async fn links(&self, Parameters(params): Parameters<LinksParams>) -> Result<String, String> {
        crate::tools::links::run(params).await
    }

    #[tool(
        name = "spider_transform",
        description = "Convert raw HTML to markdown, plain text, or XML. No network requests — pure offline transformation."
    )]
    async fn transform(
        &self,
        Parameters(params): Parameters<TransformParams>,
    ) -> Result<String, String> {
        crate::tools::transform::run(params)
    }
}

#[tool_handler]
impl ServerHandler for SpiderMcpServer {}
