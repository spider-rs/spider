# spider_mcp

MCP (Model Context Protocol) server that exposes [Spider](https://github.com/spider-rs/spider) web crawler capabilities as tools for AI assistants.

## Tools

| Tool | Description |
|------|-------------|
| `spider_scrape` | Fetch a web page and return content as markdown, text, HTML, or XML |
| `spider_crawl` | Crawl a website discovering linked pages with configurable depth/limit |
| `spider_links` | Extract all links from a page without fetching content |
| `spider_transform` | Convert raw HTML to markdown/text/XML (offline, no network) |

## Install

```bash
cargo install spider_mcp
```

Or build from source:

```bash
cargo build -p spider_mcp --release
```

### Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `chrome` | yes | Chrome/CDP rendering for JavaScript-heavy sites |
| `chrome_screenshot` | no | Page screenshot capture |
| `smart` | no | Smart mode (hybrid Chrome + HTTP) |
| `search_serper` | no | Web search via Serper |
| `search_brave` | no | Web search via Brave |
| `full` | no | All features |

Minimal build (HTTP only, no Chrome):

```bash
cargo install spider_mcp --no-default-features
```

## Usage

### Claude Code

Add to `~/.claude/settings.json`:

```json
{
  "mcpServers": {
    "spider": {
      "command": "spider-mcp",
      "args": [],
      "env": {}
    }
  }
}
```

### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "spider": {
      "command": "spider-mcp",
      "args": []
    }
  }
}
```

### With Chrome rendering

Point to a running Chrome instance:

```json
{
  "mcpServers": {
    "spider": {
      "command": "spider-mcp",
      "args": [],
      "env": {
        "SPIDER_CHROME_URL": "ws://localhost:9222"
      }
    }
  }
}
```

## CLI Options

```
spider-mcp [OPTIONS]

Options:
  --log-level <LEVEL>  Log level: error, warn, info, debug, trace (default: warn)
```

Logs go to stderr (stdout is the MCP transport channel).

## Tool Examples

### spider_scrape

Fetch a page as markdown:

```json
{
  "url": "https://example.com",
  "return_format": "markdown"
}
```

With Chrome rendering and wait conditions:

```json
{
  "url": "https://example.com",
  "headless": true,
  "wait_for": "#content",
  "return_format": "text"
}
```

### spider_crawl

Crawl up to 5 pages:

```json
{
  "url": "https://example.com",
  "limit": 5,
  "depth": 2,
  "return_format": "markdown"
}
```

### spider_links

Extract links from a page:

```json
{
  "url": "https://example.com",
  "subdomains": true
}
```

### spider_transform

Convert HTML to markdown (no network):

```json
{
  "html": "<h1>Hello</h1><p>World</p>",
  "return_format": "markdown"
}
```

## License

MIT
