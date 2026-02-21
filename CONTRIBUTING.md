# Contributing to Spider

Thanks for taking the time to contribute! Spider is an open-source web crawler and scraper built in Rust, and contributions of all kinds are welcome — bug fixes, new features, documentation improvements, and performance optimizations.

## Getting Started

### Prerequisites

- [**Rust**](https://rustup.rs/) (edition 2021, stable toolchain)
- **Git**
- For Chrome/CDP features: a local Chrome or Chromium installation

### Setup

```bash
git clone https://github.com/spider-rs/spider.git
cd spider
cargo build
```

Run the test suite:

```bash
cargo test
```

For Chrome-specific tests:

```bash
cargo test --features chrome
```

### Project Structure

```
spider/              Core crawling library (Website, Page, Configuration)
spider_agent/        AI-powered web automation agent (LLM integration, skills)
spider_cli/          Command-line interface
spider_worker/       Distributed crawling worker
spider_utils/        CSS/XPath scraping utilities
spider_agent_html/   HTML extraction for agent pipelines
spider_agent_types/  Shared types for the agent crate
examples/            64 runnable examples covering most features
benches/             Criterion benchmarks
```

## How to Contribute

### Reporting Bugs

Open an issue at [github.com/spider-rs/spider/issues](https://github.com/spider-rs/spider/issues) with:

- Steps to reproduce
- Expected vs. actual behavior
- Spider version, Rust version, and OS
- A minimal code example or URL that triggers the bug

### Suggesting Features

Open an issue describing:

- The use case and why it matters
- A proposed API or behavior
- A practical example showing how it would be used

### Pull Requests

1. Fork the repo and create a branch from `master`
2. Make your changes
3. Add or update tests for your changes
4. Run `cargo test` and ensure everything passes
5. Run `cargo fmt` to format your code
6. Open a pull request against `master`

#### PR Guidelines

- **One PR per feature or fix** — keep changes focused
- **Add tests** — new features and bug fixes need test coverage. If testing is difficult, mention it in the PR and we'll help
- **Update docs** — if your change affects public API or behavior, update the relevant README or doc comments
- **Write clear commit messages** — describe what changed and why
- **Keep PRs small** — smaller PRs are easier to review and merge faster

### Feature Flags

Spider uses feature flags extensively. When adding new functionality:

- Gate optional dependencies behind a feature flag
- Add the flag to `spider/Cargo.toml` with a descriptive comment
- Document the flag in the crate's README if it's user-facing
- Make sure the default feature set still compiles: `cargo check --no-default-features`

### Testing

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p spider
cargo test -p spider_cli

# Run tests with a specific feature
cargo test --features chrome
cargo test --features cache

# Run benchmarks
cargo bench
```

## Code Style

- Follow standard Rust conventions (`cargo fmt`, `cargo clippy`)
- Use descriptive variable and function names
- Keep functions focused — prefer small functions over large ones
- Add doc comments (`///`) to public APIs

## Questions?

- Open a [GitHub Discussion](https://github.com/spider-rs/spider/discussions) for questions or ideas
- Join [Discord](https://discord.spider.cloud) for real-time help

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
