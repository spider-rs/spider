# CHANGELOG

## Unreleased

1. chore: fix clippy warnings and formatting across workspace
1. chore: remove cargo dependabot for Rust (too noisy), keep github-actions only
1. chore(deps): bump flexbuffers 2 -> 25, async-openai 0.32 -> 0.33
1. docs: rewrite README, add issue/PR templates, security policy, CI workflows
1. docs: add Contributor Covenant v2.1 Code of Conduct
1. docs: rewrite CONTRIBUTING.md and add zero-config quick start to README

## v2.45.22

1. fix(cache): proper HTTP staleness for Chrome-cached pages
1. fix(cache): Period policy bypasses HTTP `is_stale` for Chrome-rendered pages

## v2.45.21

1. fix(cache): enable `cache_chrome_hybrid_mem` feature for Chrome cache writes

## v2.45.19

1. fix(chrome): default `no_sandbox()` for headless BrowserConfig (#354)

## v2.45.18

1. feat(agent): per-round model pool routing for cost-optimized automation
1. test: comprehensive crawler-test.com integration suite (302 tests, 408 URLs)

## v2.45.17

1. perf(agent): skip model scoring for pools with 2 or fewer models

## v2.45.16

1. test(agent): comprehensive multi-LLM router reliability tests

## v2.45.15

1. feat(cli): add `--wait-for` capabilities to spider_cli (#352)

## v2.45.14

1. fix: Chrome mode honors `wait_for` config for networkIdle before HTML extraction

## v2.45.13

1. fix: smart mode lifecycle waiting -- match Chrome coverage without re-fetch

## v2.45.12

1. fix: auto-retry www. URLs on SSL protocol errors -- strip www prefix

## v2.45.11

1. fix: reject empty HTML from all cache and seeded resource paths

## v2.45.10

1. fix: invalidate empty HTML shell responses on cache read path

## v2.45.9

1. feat: deferred Chrome -- cache-only crawl phase before browser launch

## v2.45.8

1. feat: cache-first fast path -- skip browser/HTTP when cache has data

## v2.45.7

1. fix: add `Website::with_hedge()` forwarding method

## v2.45.5

1. feat: add work-stealing (hedged requests) for slow crawl requests
1. fix: skip HTML-specific heuristics in cache empty check for non-HTML content
1. fix: skip caching empty/near-empty HTML responses

## v2.45.3

1. fix(uring_fs): fix compile errors when io_uring feature is enabled

## v2.45.2

1. perf(io_uring): add StreamingWriter for streaming file I/O

## v2.45.1

1. feat(io_uring): expand io_uring integration for file I/O
1. refactor(agent): extract types and HTML cleaning into `spider_agent_types` and `spider_agent_html` crates

## v2.45.0

1. chore(deps): bump hashbrown 0.15 → 0.16
1. chore(deps): bump strum 0.26 → 0.27
1. chore(deps): bump thiserror 1 → 2
1. chore(deps): bump indexmap 1 → 2
1. chore(deps): bump async-openai 0.29 → 0.32
1. chore(deps): bump tiktoken-rs 0.7 → 0.9
1. chore(deps): bump sysinfo 0.35 → 0.38
1. chore(deps): bump quick-xml 0.38 → 0.39
1. chore(deps): bump criterion 0.5 → 0.8
1. fix(openai): update async-openai imports for v0.32 types::chat module
1. fix(page): use resolver().resolve_element() for quick-xml 0.39

## v2.44.42

1. feat(agent): integrate llm_models_spider v0.1.9 with smart model selection
1. perf(website): use `take()` instead of `clone()` for subdomain base URL

## v2.44.41

1. fix(website): use page's own URL for relative link resolution on subdomains (#351)

## v2.44.40

1. docs: add all missing feature flags (`spider_cloud`, `agent`, `search`, `webdriver`, `wreq`, `adblock`, `simd`, `tracing`, etc.)
1. docs: add Spider Cloud and Chrome rendering integration examples

## v2.44.39

1. refactor(agent): move all skill content to `spider_skills` crate (110 skills via `include_str!`)

## v2.44.38

1. feat(agent): add L15-L48 not-a-robot skills (34 new levels)
1. feat(agent): add NEST engine loop for recursive nested challenges
1. feat(agent): add CIRCLE engine loop for drawing challenges
1. feat(agent): add haiku benchmark for agent evaluation

## v2.44.37

1. fix: add `should_use_chrome_ai` and `use_chrome_ai` to stub `RemoteMultimodalConfigs` (chrome-only builds)

## v2.44.36

1. feat(agent): add L8-L14 not-a-robot skills (license plate, nested, whack-a-mole, waldo, chihuahuas, reverse, affirmations)
1. feat(agent): add WAM engine loop for whack-a-mole challenges
1. feat(agent): Chrome AI element probe improvements

## v2.44.33

1. feat(cache): optimize automation caching for skip-browser flows

## v2.44.30

1. feat: add spider cloud end-to-end examples

## v2.44.29

1. feat(agent): improve remote multimodal automation reliability

## v2.44.28

1. feat(agent): expose optional automation reasoning in metadata

## v2.44.26

1. feat(spider_cli): add runtime `--http` and `--headless` mode controls

## v2.44.25

1. feat(agent): dual-model routing with per-endpoint URL and API key configuration
1. feat(agent): extraction-only mode optimization for single-round data extraction

## v2.44.21

1. fix: feature flag compilation across `wreq`, `agent_full`, and `cache` combos
1. fix: `agent_full` memvid `!Send` compat via `spawn_blocking`
1. fix: `cache_chrome_hybrid` GEMINI_CLIENT lazy_static cfg gate
1. fix: `detect_cf_turnstyle` cfg gate (chrome, not real_browser)

## v2.44.20

1. fix: broken `chrome` feature — missing `relevant` field and cfg gate on `detect_cf_turnstyle` (#349)

## v2.44.18

1. feat(agent): add URL-level relevance pre-filter for crawling — classify URLs via text model before fetching, skip irrelevant ones
1. feat(agent): add `url_prefilter` and `relevance_gate` configuration

## v2.44.17

1. perf: trie `entry_ref` optimization (-11% lookup time)
1. perf: robot parser hoisted `to_lowercase` (-13% parse time)
1. perf: `prepare_url` byte indexing optimization

## v2.44.16

1. feat(agent): add relevance gate for remote multimodal — LLM returns `relevant: true|false`, irrelevant pages get wildcard budget refunded

## v2.44.15

1. perf: trie 49-70% faster via optimized hot paths
1. perf: robot parser 50% faster
1. perf: HTML cleaner selector merging
1. chore: add criterion benchmarks for trie, robot parser, and URL preparation

## v2.44.13

1. feat(spider): add `spider_cloud` integration and S3 skills loading

## v2.44.12

1. feat(spider_agent): add dual-model routing (vision + text model selection)
1. feat(spider_agent): add long-term experience memory for automation sessions

## v2.44.10

1. feat(spider_agent): improve skill triggers and board reading for web challenges

## v2.44.9

1. feat(spider_agent): add Claude-optimized automation features
1. feat(agent): add `pre_evaluate` field on skills — engine runs JS before LLM inference

## v2.44.8

1. feat(spider_agent): add concurrent page spawning with `OpenPage` action

## v2.44.7

1. feat(spider_agent): integrate `llm_models_spider` for auto-updated vision model detection

## v2.44.6

1. feat(spider): enable HTTP extraction without `agent_chrome` feature

## v2.44.5

1. feat(agent): enhance CAPTCHA handling and lock system prompt
1. feat(agent): Chrome AI (Gemini Nano) integration — on-device LLM via `LanguageModel` API

## v2.44.3

1. feat(agent): consolidate automation into `spider_agent` with seamless feature integration
1. feat: granular `AutomationUsage` tracking (tokens, api_calls, screenshots)
1. feat(spider_agent): add usage limits, custom tools, and granular tracking

## v2.43.22

1. feat(automation): add `api_calls` tracking to AutomationUsage
1. feat(page): make `remote_multimodal_usage` and `extra_remote_multimodal_data` work for HTTP-only crawls (not just Chrome)
1. feat(page): add `usage` field to AutomationResults for per-result token tracking

## spider_agent v0.5.1

1. feat(automation): add `api_calls` tracking to AutomationUsage for counting LLM API calls

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
