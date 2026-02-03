# CHANGELOG

## Unreleased

## v2.43.21

1. chore(spider): update spider_agent dependency to 0.5

## spider_agent v0.5.0

1. feat(actions): add complete WebAutomation parity with 17 new ActionType variants
   - Click variants: ClickAll, ClickPoint, ClickHold, ClickHoldPoint, ClickDrag, ClickDragPoint, ClickAllClickable
   - Wait variants: WaitFor, WaitForWithTimeout, WaitForNavigation, WaitForDom, WaitForAndClick
   - Scroll variants: ScrollX, ScrollY, InfiniteScroll
   - Input: Fill (clear + type)
   - Chain control: ValidateChain
1. feat(automation): add PromptUrlGate for URL-based prompt/config overrides
   - Exact URL matching
   - Path-prefix matching (case-insensitive)
   - Per-URL config overrides
1. feat(browser): add comprehensive browser methods for all new action types
   - click_all, click_point, click_hold, click_hold_point
   - click_drag, click_drag_point, click_all_clickable
   - wait_for_timeout, wait_for_navigation, wait_for_dom, wait_and_click
   - scroll_x, scroll_y, infinite_scroll
   - fill, find_elements, get_element_bounds
1. feat(config): add system_prompt, system_prompt_extra, user_message_extra to AutomationConfig

## v2.43.20

1. fix(spider): fix doctest and update chromey for adblock compatibility
1. fix(search): use reqwest::Client directly for cache feature compatibility
1. chore(spider): update spider_agent dependency to 0.4

## spider_agent v0.4.0

1. feat(cache): add SmartCache with size-aware LRU eviction and TTL expiration
1. feat(executor): add ChainExecutor for parallel step execution with response caching
1. feat(executor): add BatchExecutor for efficient batch processing
1. feat(executor): add PrefetchManager for predictive page loading
1. feat(router): add ModelRouter for smart model selection based on task complexity
1. feat(llm): add MessageContent helper methods (as_text, full_text, is_text, has_images)
1. fix(config): default ModelPolicy now allows High tier routing

## spider_agent v0.3.0

1. feat(automation): add comprehensive automation module with action chains
1. feat(automation): add self-healing selector cache with LRU eviction
1. feat(automation): add content analysis for smart screenshot decisions
1. feat(automation): add configurable model policies and retry strategies

## spider_agent v0.2.0

1. feat(memory): enhance memory with URL, action, and extraction history
1. feat(webdriver): add webdriver support via thirtyfour
1. feat(browser): add chrome browser and temp storage support

## v2

### Multimodal AI Integration

1. feat(openai): OpenAI integration for dynamic browser scripting and automation
1. feat(gemini): Gemini AI support for intelligent web interaction
1. feat(solver): built-in Gemini Nano support for web challenge solving
1. feat(chrome): remote multimodal web automation with vision capabilities
1. feat(automation): token usage tracking for LLM-powered extraction

### Agentic Web Automation

1. feat(automation): simplified agentic APIs - `act()`, `observe()`, `extract()`
1. feat(automation): agentic memory for multi-round automation sessions
1. feat(automation): prompt-based website configuration
1. feat(automation): selector cache with self-healing and LRU eviction
1. feat(automation): structured outputs with ExtractionSchema
1. feat(automation): autonomous agent with action chaining and error recovery
1. feat(automation): intelligent screenshot detection based on content analysis
1. feat(automation): byte-size-based smart HTML cleaning for optimal performance
1. feat(llm_json): robust JSON parsing from LLM outputs with thinking model support

### WebDriver Support

1. feat(webdriver): WebDriver support via thirtyfour crate
1. feat(webdriver): Selenium Grid and remote browser connectivity
1. feat(webdriver): multi-browser support (Chrome, Firefox, Edge)
1. feat(webdriver): stealth mode with spider_fingerprint integration
1. feat(webdriver): automation script support
1. feat(webdriver): screenshot capabilities

### Web Search Integration

1. feat(search): web search integration with multiple providers
1. feat(search): Serper.dev, Brave Search, Bing, and Tavily AI Search support
1. feat(search): `search_and_extract()` for combined search + data extraction
1. feat(search): `research()` method for multi-source topic research

### spider_agent Crate

1. feat(agent): standalone concurrent-safe multimodal agent crate
1. feat(agent): feature-gated LLM providers (OpenAI, OpenAI-compatible)
1. feat(agent): feature-gated search providers (Serper, Brave, Bing, Tavily)
1. feat(agent): Chrome browser automation support
1. feat(agent): smart caching with LRU eviction and TTL expiration
1. feat(agent): high-performance chain executor with parallel step support
1. feat(agent): batch processing and prefetch management
1. feat(agent): smart model routing based on task complexity

### Browser & Chrome Enhancements

1. feat(chrome): remote cache support (disk and memory)
1. feat(chrome): skip browser mode with smart HTML cleaning
1. feat(chrome): adblock integration via chromey
1. feat(chrome): idle network detection for page load completion
1. feat(chrome): auto geo-detection
1. feat(chrome): max page bytes control
1. feat(smart): improved smart mode with JS rendering detection
1. feat(smart): Imperva and sessionStorage detection handling

### Anti-Bot & Security

1. feat(antibot): anti-bot detection capabilities
1. feat(fingerprint): centralized browser fingerprint emulation
1. feat(fingerprint): header emulation for stealth
1. feat(solver): deterministic and AI-powered web challenge solvers
1. feat(solver): Lemin solver support
1. feat(firewall): firewall integration for request filtering

### Data Processing

1. feat(transform): HTML transformation crate with spider_transformations
1. feat(css_scraping): CSS/XPath scraping with spider_utils
1. feat(page): metadata extraction from pages
1. feat(website): seeded page link and metadata extraction
1. feat(decentralized): improved decentralized crawling with remote multimodal support

### Performance & Infrastructure

1. feat(cache): hybrid caching (Chrome + HTTP cache)
1. feat(cache): memory and disk cache options
1. feat(cmd): command-line crawling support
1. feat(disk): shared state multi-profiling
1. perf(website): reduced unnecessary clones and allocations
1. chore(chrome): stabilized concurrent screenshot handling

## v1.98.0

1. feat(whitelist): whitelist routes to only crawl.

## v1.85.0

1. feat(openai): use OpenAI to dynamically drive the browser.

## v1.84.1

1. feat(chrome): add chrome_headless_new flag

## v1.83.11

1. chore(chrome): add wait_for events

## v1.60.0

1. feat(smart): add smart mode feature flag (HTTP until JS Rendering is needed per page)

## v1.50.1

1. feat(cron): add cron feature flag [#153]

## v1.36.0

1. feat(sync): subscribe to page updates to perform async handling of data

## v1.31.0

1. feat(js): add init of script parsing

## v1.30.5

1. feat(worker): add tls support

## v1.30.3

1. chore(request): add custom domain redirect policy

## v1.30.2

1. chore(glob): fix glob crawl establish

## v1.30.1

1. chore(crawl): fix crawl asset detection and trailing start

## v1.29.0

1. feat(fs): add temp storage resource handling (#112)
1. feat(url-glob): URL globbing (#113) thanks to [@roniemartinez](https://github.com/roniemartinez))

## v1.28.5

1. chore(request): fix resource success handling

## v1.28.0

1. feat(proxies): add proxy support

## v1.27.2

1. feat(decentralization): add workload split

## v1.19.36

1. perf(crawl): add join handle task management

## v1.19.26

1. perf(links): add fast pre serialized url anchor link extracting and reduced memory usage
1. perf(links): fix case sensitivity handling
1. perf(crawl): reduce memory usage on link gathering
1. chore(crawl): remove `Website.reset` method and improve crawl handling resource usage ( `reset` not needed now )
1. chore(crawl): add heap usage of links visited
1. perf(crawl): massive scans capability to utilize more cpu
1. feat(timeout): add optional `configuration.request_timeout` duration
1. build(tokio): remove unused `net` feature
1. chore(docs): add missing scrape section

## v1.10.7

- perf(req): enable brotli
- chore(tls): add ALPN tls defaults
- chore(statics): add initial static media ignore
- chore(robots): add shared client handling across parsers
- feat(crawl): add subdomain and tld crawling

## v1.6.1

- perf(links): filter dup links after async batch
- chore(delay): fix crawl delay thread groups
- perf(page): slim channel page sending required props

## v1.5.3

- feat(regex): add optional regex black listing

## v1.5.0

- chore(bin): fix bin executable [#17](https://github.com/madeindjs/spider/pull/17/commits/b41e25fc507c6cd3ef251d2e25c97b936865e1a9)
- feat(cli): add cli separation binary [#17](https://github.com/madeindjs/spider/pull/17/commits/b41e25fc507c6cd3ef251d2e25c97b936865e1a9)
- feat(robots): add robots crawl delay respect and ua assign [#24](https://github.com/madeindjs/spider/pull/24)
- feat(async): add async page body gathering
- perf(latency): add connection re-use across request [#25](https://github.com/madeindjs/spider/pull/25)

## v1.4.0

- feat(cli): add cli ability ([#16](https://github.com/madeindjs/spider/pull/16) thanks to [@j-mendez](https://github.com/j-mendez))
- feat(concurrency): dynamic concurrent cpu defaults ([#15](https://github.com/madeindjs/spider/pull/15) thanks to [@j-mendez](https://github.com/j-mendez))
- docs: add a changelog

## v1.3.1

- fix(crawl): fix field type ([#14](https://github.com/madeindjs/spider/pull/14) thanks to [@j-mendez](https://github.com/j-mendez))

## v1.3.0

- feat(crawl): callback to run when link is found ([#13](https://github.com/madeindjs/spider/pull/13) thanks to [@j-mendez](https://github.com/j-mendez))

## v1.2.0

- Add User Agent configuration ([#5](https://github.com/madeindjs/spider/pull/5) thanks to [@Dragnucs](https://github.com/Dragnucs))
- Add polite delay ([#6](https://github.com/madeindjs/spider/pull/6) thanks to [@Dragnucs](https://github.com/Dragnucs) )

## v1.1.3

- Handle page get errors ([#4](https://github.com/madeindjs/spider/pull/4) thanks to [@Dragnucs](https://github.com/Dragnucs))
- Fix link resolution ([#3](https://github.com/madeindjs/spider/pull/3) thanks to [@Dragnucs](https://github.com/Dragnucs))
