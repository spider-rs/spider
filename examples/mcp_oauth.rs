//! Connect to a remote MCP server with OAuth 2.1 — the same flow ChatGPT and
//! Claude use for custom connectors.
//!
//! Runs the full authorization-code + PKCE handshake against a Model Context
//! Protocol server (discovery, dynamic client registration, the browser consent
//! step, then the token exchange), opens an authenticated session, and lists the
//! tools the token unlocks.
//!
//! `cargo run --example mcp_oauth`
//!
//! Set `MCP_SERVER` to point at another server (default: <https://mcp.spider.cloud/mcp>).

use std::error::Error;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const DEFAULT_SERVER: &str = "https://mcp.spider.cloud/mcp";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = std::env::var("MCP_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.into());
    let http = reqwest::Client::new();

    // 1. Discover the authorization server from the protected-resource metadata.
    let auth = discover(&http, &server).await?;
    println!("authorization server: {}", auth.issuer);

    // 2. Bind a loopback listener for the redirect, then register this client.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect_uri = format!("http://{}/callback", listener.local_addr()?);
    let client_id = register(&http, &auth.registration_endpoint, &redirect_uri).await?;

    // 3. Generate the PKCE pair and build the authorization URL.
    let verifier = random_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token();
    let authorize_url = http
        .get(&auth.authorization_endpoint)
        .query(&[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("scope", "mcp"),
            ("resource", server.as_str()),
        ])
        .build()?
        .url()
        .to_string();

    // 4. Send the user to approve, then catch the redirect on the listener.
    println!("\nopen this URL to authorize:\n{authorize_url}\n");
    open_browser(&authorize_url);
    let code = wait_for_code(&listener, &state).await?;

    // 5. Exchange the code for an access token, proving possession of the verifier.
    let token: Value = http
        .post(&auth.token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form_urlencode(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", client_id.as_str()),
            ("resource", server.as_str()),
        ]))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let access_token = token["access_token"]
        .as_str()
        .ok_or("token response had no access_token")?;
    println!("received access token ({} chars)", access_token.len());

    // 6. Open an MCP session and list the tools the token unlocks.
    let (init, session) = mcp(&http, &server, access_token, None, &json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "spider-mcp-oauth-example", "version": "1.0" }
        }
    }))
    .await?;
    if let Some(name) = init["result"]["serverInfo"]["name"].as_str() {
        println!("connected to {name}");
    }
    notify(&http, &server, access_token, session.as_deref()).await?;

    let (tools, _) = mcp(&http, &server, access_token, session.as_deref(), &json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/list"
    }))
    .await?;
    if let Some(list) = tools["result"]["tools"].as_array() {
        println!("\n{} tools available:", list.len());
        for tool in list {
            if let Some(name) = tool["name"].as_str() {
                println!("  - {name}");
            }
        }
    }

    Ok(())
}

struct AuthServer {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: String,
}

/// Walk RFC 9728 protected-resource metadata to the RFC 8414 authorization-server metadata.
async fn discover(http: &reqwest::Client, server: &str) -> Result<AuthServer, Box<dyn Error>> {
    let origin = origin_of(server)?;
    let resource: Value = http
        .get(format!("{origin}/.well-known/oauth-protected-resource/mcp"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let issuer = resource["authorization_servers"][0]
        .as_str()
        .ok_or("no authorization server advertised")?
        .to_string();

    let meta: Value = http
        .get(format!("{issuer}/.well-known/oauth-authorization-server"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(AuthServer {
        authorization_endpoint: field(&meta, "authorization_endpoint")?,
        token_endpoint: field(&meta, "token_endpoint")?,
        registration_endpoint: field(&meta, "registration_endpoint")?,
        issuer,
    })
}

/// RFC 7591 dynamic client registration — returns a fresh public `client_id`.
async fn register(
    http: &reqwest::Client,
    endpoint: &str,
    redirect_uri: &str,
) -> Result<String, Box<dyn Error>> {
    let registered: Value = http
        .post(endpoint)
        .json(&json!({
            "client_name": "spider mcp_oauth example",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    field(&registered, "client_id")
}

/// Block on the loopback listener until the browser delivers `?code=...`, validating `state`.
async fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String, Box<dyn Error>> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut buf = [0u8; 2048];
        let n = stream.read(&mut buf).await?;
        let target = String::from_utf8_lossy(&buf[..n])
            .split_whitespace()
            .nth(1)
            .unwrap_or("/")
            .to_string();

        let page = "<html><body>Authorization complete — you can close this tab.</body></html>";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{page}",
                    page.len()
                )
                .as_bytes(),
            )
            .await;

        if !target.starts_with("/callback") {
            continue;
        }
        let params = query_pairs(&target);
        let get = |key: &str| params.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());

        if let Some(error) = get("error") {
            return Err(format!("authorization denied: {error}").into());
        }
        if get("state").as_deref() != Some(state) {
            return Err("state mismatch — discarding response".into());
        }
        if let Some(code) = get("code") {
            return Ok(code);
        }
    }
}

/// Send one MCP request over streamable HTTP. Returns the JSON-RPC response and any session id.
async fn mcp(
    http: &reqwest::Client,
    url: &str,
    token: &str,
    session: Option<&str>,
    body: &Value,
) -> Result<(Value, Option<String>), Box<dyn Error>> {
    let resp = session
        .into_iter()
        .fold(mcp_request(http, url, token).json(body), |req, id| {
            req.header("mcp-session-id", id)
        })
        .send()
        .await?
        .error_for_status()?;
    let session_id = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    Ok((parse_jsonrpc(&resp.text().await?)?, session_id))
}

/// Confirm initialization (a fire-and-forget JSON-RPC notification).
async fn notify(
    http: &reqwest::Client,
    url: &str,
    token: &str,
    session: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    session
        .into_iter()
        .fold(
            mcp_request(http, url, token).json(&json!({
                "jsonrpc": "2.0", "method": "notifications/initialized"
            })),
            |req, id| req.header("mcp-session-id", id),
        )
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

fn mcp_request(http: &reqwest::Client, url: &str, token: &str) -> reqwest::RequestBuilder {
    http.post(url)
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
}

// --- small helpers ---------------------------------------------------------

/// Streamable HTTP answers as JSON or as an SSE stream; pull the payload from either.
fn parse_jsonrpc(text: &str) -> Result<Value, Box<dyn Error>> {
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            return Ok(serde_json::from_str(data.trim())?);
        }
    }
    Ok(serde_json::from_str(text)?)
}

fn field(value: &Value, key: &str) -> Result<String, Box<dyn Error>> {
    Ok(value[key]
        .as_str()
        .ok_or_else(|| format!("response missing `{key}`"))?
        .to_string())
}

fn query_pairs(target: &str) -> Vec<(String, String)> {
    target
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or("")
        .split('&')
        .filter_map(|pair| pair.split_once('=').map(|(k, v)| (decode(k), decode(v))))
        .collect()
}

/// Minimal percent-decoding — enough for an OAuth callback's `code`/`state`.
fn decode(input: &str) -> String {
    let bytes = input.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes.get(i..i + 3) {
            Some([b'%', a, b]) => match u8::from_str_radix(&format!("{}{}", *a as char, *b as char), 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                }
                Err(_) => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Serialize key/value pairs as `application/x-www-form-urlencoded`.
fn form_urlencode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encode a form value, leaving the RFC 3986 unreserved set untouched.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn origin_of(url: &str) -> Result<String, Box<dyn Error>> {
    let (scheme, rest) = url
        .split_once("://")
        .filter(|(s, _)| *s == "http" || *s == "https")
        .ok_or("MCP server must be an http(s) URL")?;
    let host = rest.split('/').next().unwrap_or(rest);
    Ok(format!("{scheme}://{host}"))
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("OS random number generator");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Best effort: open the URL in the default browser. The URL is also printed, so
/// failure here is harmless.
fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "start"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(url).spawn();
}
