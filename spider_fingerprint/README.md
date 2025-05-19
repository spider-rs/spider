# spider_fingerprint

A Rust crate to generate stealth JavaScript that spoofs browser fingerprinting features. Useful for emulateting real browser profiles across different platforms.
It is recommended to use this project with [headless-browser](https://github.com/spider-rs/headless-browser) for real profiles and the latest chrome versions.

## Purpose

- Mimic real user fingerprints using static profiles
- Help avoid common browser automation detection methods
- Generate scripts for injection into browser environments

## Features

- Tiered spoofing levels (basic to full)
- WebGL and GPU spoofing (WIP)
- `navigator.userAgentData` high entropy value support
- Plugin and mimeType spoofing
- Optional mouse and viewport spoofing
- Platform-specific variants (macOS, Windows, Linux)

## Example

```rust
use spider_fingerprint::{build_stealth_script, builder::{Tier, AgentOs}};

let script = build_stealth_script(Tier::Full, AgentOs::Mac);
// Inject `script` into a browser context
```

## Spoofing Tiers

This crate provides multiple spoofing levels depending on the desired realism and complexity.

```md

| Tier            | Description                                               |
|-----------------|-----------------------------------------------------------|
| `Basic`         | Chrome props, WebGL spoofing, plugins/mimeTypes           |
| `BasicNoWebgl`  | Same as Basic but skips WebGL spoofing                    |
| `Mid`           | Adds WebDriver hiding                                     |
| `Full`          | All spoofing including WebGPU adapter spoof               |
```

## Configuration

You can override the default Chrome versions with the env configs:

```sh
CHROME_VERSION=135 
CHROME_NOT_A_BRAND_VERSION="99.0.0.0" 
```

## License

MIT
