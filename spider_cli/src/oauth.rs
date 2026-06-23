//! Browser-based OAuth 2.1 + PKCE sign-in for Spider Cloud.
//!
//! `spider login` runs this flow: it discovers the authorization server from the
//! Spider MCP endpoint, registers a client, opens the browser for consent, then
//! exchanges the authorization code for a freshly provisioned Spider Cloud API
//! key. The key is delivered straight to a loopback callback on this machine and
//! stored in `~/.spider/credentials`.
//!
//! Set `SPIDER_MCP_SERVER` to target a different deployment.

use std::error::Error;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const DEFAULT_SERVER: &str = "https://mcp.spider.cloud/mcp";

/// Sign in through the browser and return a provisioned Spider Cloud API key.
pub async fn login() -> Result<String, Box<dyn Error>> {
    let server = std::env::var("SPIDER_MCP_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.into());
    let http = reqwest::Client::new();

    // Discover the authorization server, then register on a loopback redirect.
    let auth = discover(&http, &server).await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect_uri = format!("http://{}/callback", listener.local_addr()?);
    let client_id = register(&http, &auth.registration_endpoint, &redirect_uri).await?;

    // Build the PKCE pair and the authorization URL.
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

    eprintln!("Opening your browser to sign in to Spider Cloud...");
    eprintln!("If it does not open automatically, visit:\n{authorize_url}\n");
    open_browser(&authorize_url);

    // Wait for the redirect, then exchange the code for an API key.
    let code = wait_for_code(&listener, &state).await?;
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

    token["spider_api_key"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "the authorization server did not return an API key".into())
}

struct AuthServer {
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
            "client_name": "Spider CLI",
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

        let page = "<html><body>Signed in to Spider Cloud — you can close this tab and return to your terminal.</body></html>";
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

fn decode(input: &str) -> String {
    percent_decode_str(&input.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned()
}

fn form_urlencode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                utf8_percent_encode(key, NON_ALPHANUMERIC),
                utf8_percent_encode(value, NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn origin_of(url: &str) -> Result<String, Box<dyn Error>> {
    let (scheme, rest) = url
        .split_once("://")
        .filter(|(scheme, _)| *scheme == "http" || *scheme == "https")
        .ok_or("MCP server must be an http(s) URL")?;
    Ok(format!("{scheme}://{}", rest.split('/').next().unwrap_or(rest)))
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("OS random number generator");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Best effort: open the URL in the default browser (the URL is also printed).
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
