# Authenticated Page CLI 报告

## 背景

这一阶段工作的重点，是把现有“基于本机浏览器登录态的单页抓取能力”整理成一个可复用的 CLI 能力。

最初这套能力是通过 example 形式验证的，主要用于确认以下几点：

- Ubuntu Desktop 上本机浏览器 profile 复用可行
- 通过 Chromium 渲染抓取登录态页面可行
- 能保存渲染后的 HTML
- 能从渲染后的 HTML 中抽取标题和正文文本

这一阶段的目标，是把这套能力迁入 `spider_cli`，从而后续可以直接依赖编译好的可执行程序，而不再依赖 example 入口或源码使用流程。

## 目标

本阶段希望达成的结果是：

- 保持“本机 Chromium profile 复用”这条技术路线不变
- 尽量不修改 `spider` 核心 crate
- 把登录态单页抓取能力变成 `spider_cli` 的正式子命令
- 支持统一输出目录
- 支持在 HTML 和文本之外，顺便下载页面图片

## 结果

当前范围内的工作已经完成。

`spider_cli` 现在已经包含一个新的子命令：

```bash
spider authenticated-page
```

这个子命令支持：

- 准备或复用本地 Chromium profile
- 抓取单个登录态页面
- 保存渲染后的 HTML
- 抽取标题和主要文本内容
- 下载页面图片
- 把所有结果统一写入一个输出目录

## 已实现功能

### 1. 登录态浏览器 profile 复用

CLI 现在可以启动或复用一个专用的本地 Chromium profile，并通过 CDP 连接到它。

目前支持的参数包括：

- `--prepare-profile`
- `--chrome-bin`
- `--chrome-user-data-dir`
- `--chrome-profile-dir`
- `--chrome-connection-url`
- `--chrome-debugging-port`
- `--chrome-start-url`
- `--chrome-headless`
- `--chrome-extra-args`

### 2. 登录态单页抓取

CLI 现在可以打开目标页面，并在浏览器渲染完成后抓取页面内容。

目前支持的参数包括：

- `--url`
- `--output-dir`
- `--output-html`
- `--output-json`
- `--title-selectors`
- `--content-selectors`
- `--image-selectors`
- `--user-agent`
- `--accept-language-header`
- `--referer-url`
- `--cookie`

### 3. 统一结果目录

抓取结果现在会统一写入一个目录。

默认目录结构如下：

```text
authenticated_page_output/
  page.html
  page_extracted.json
  images/
```

### 4. 图片下载

登录态抓取流程现在会从渲染后的 HTML 中提取图片地址，并把图片下载到结果目录下。

如果不需要图片下载，可以关闭：

```bash
--no-download-images
```

## 新增或修改的文件

### Example 层

- 新增：[examples/authenticated_page.rs](/home/kali/Desktop/spider/examples/authenticated_page.rs)
- 删除：[examples/zhihu_cookie_login.rs](/home/kali/Desktop/spider/examples/zhihu_cookie_login.rs)
- 修改：[examples/Cargo.toml](/home/kali/Desktop/spider/examples/Cargo.toml)
- 修改：[examples/README.md](/home/kali/Desktop/spider/examples/README.md)

### CLI 层

- 新增：[spider_cli/src/authenticated_page.rs](/home/kali/Desktop/spider/spider_cli/src/authenticated_page.rs)
- 修改：[spider_cli/src/main.rs](/home/kali/Desktop/spider/spider_cli/src/main.rs)
- 修改：[spider_cli/src/options/args.rs](/home/kali/Desktop/spider/spider_cli/src/options/args.rs)
- 修改：[spider_cli/src/options/mod.rs](/home/kali/Desktop/spider/spider_cli/src/options/mod.rs)
- 修改：[spider_cli/src/options/sub_command.rs](/home/kali/Desktop/spider/spider_cli/src/options/sub_command.rs)
- 修改：[spider_cli/Cargo.toml](/home/kali/Desktop/spider/spider_cli/Cargo.toml)
- 修改：[spider_cli/README.md](/home/kali/Desktop/spider/spider_cli/README.md)

## 未改动的部分

本次 CLI 集成过程中，没有修改 `spider` 核心 crate。

也就是说，这套新能力是在 `spider_cli` 层完成的，没有去改动底层 crawler engine。

## 使用方式

### 准备可复用的登录 profile

```bash
spider authenticated-page \
  --prepare-profile \
  --chrome-bin /usr/bin/chromium \
  --chrome-user-data-dir ~/.local/share/spider/login-chrome-profile
```

### 抓取登录态页面

```bash
spider authenticated-page \
  --url 'https://example.com/protected/page' \
  --chrome-bin /usr/bin/chromium \
  --chrome-user-data-dir ~/.local/share/spider/login-chrome-profile \
  --output-dir run_output
```

### 关闭图片下载

```bash
spider authenticated-page \
  --url 'https://example.com/protected/page' \
  --chrome-bin /usr/bin/chromium \
  --chrome-user-data-dir ~/.local/share/spider/login-chrome-profile \
  --output-dir run_output \
  --no-download-images
```

## 已完成验证

本阶段已经完成以下验证：

```bash
cargo check -p spider_cli
```

```bash
cargo run -p spider_cli -- authenticated-page --help
```

```bash
cargo build -p spider_cli --release
```

```bash
target/release/spider --help
```

```bash
target/release/spider authenticated-page --help
```

## 当前限制

当前 `authenticated-page` 已经可以作为通用登录态页面抓取器使用，但它并不能保证绕过强风控或强反爬站点。

目前已经观察到的限制包括：

- 小红书会在自动化抓取开始时主动弹登录层或验证层
- 百度贴吧会返回“百度安全验证”页面，而不是真实内容页

这说明：

- 登录态 profile 复用本身是有效的
- 浏览器渲染抓取链路本身是有效的
- 但对强风控网站，站点仍然可能主动拦截自动化访问

## 部署意义

到当前阶段为止，运行这套登录态抓取功能时，已经不再需要源码。

可以直接使用 release 二进制：

- 可执行文件：`target/release/spider`
- 运行时依赖：Chromium/Chrome
- 运行时状态：一个可持久化的浏览器 profile 目录

这个 profile 目录不一定要从开发机预先拷过去，也可以在目标机器上通过第一次执行 `--prepare-profile` 后，手工登录现场生成。

## 提交状态

这一阶段的工作已经完成本地提交：

- Commit：`26e03855`
- Message：`Add authenticated page capture to spider CLI`

## 当前仍未提交的无关本地改动

工作区里目前仍有一些和本次 `authenticated-page` CLI 功能无关的改动，我没有把它们纳入这次提交：

- [Cargo.toml](/home/kali/Desktop/spider/Cargo.toml)
- [Cargo.lock](/home/kali/Desktop/spider/Cargo.lock)
- `zhihu_spider/`

这些内容不属于本次登录态 CLI 能力的范围。
