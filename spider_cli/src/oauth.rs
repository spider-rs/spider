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

/// The page shown in the browser tab once the loopback callback is received.
const CALLBACK_PAGE: &str = r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Spider CLI</title><style>:root{color-scheme:dark}html,body{height:100%}body{margin:0;display:flex;align-items:center;justify-content:center;background:#0a0e14;color:#e6e9ef;font:14px/1.6 ui-monospace,SFMono-Regular,Menlo,monospace}.card{text-align:center;padding:40px}.mark{width:52px;height:52px;margin:0 auto 22px;border-radius:14px;background:#fff;display:flex;align-items:center;justify-content:center}.mark svg{width:30px;height:28px}h1{font-size:15px;font-weight:600;margin:0 0 8px;letter-spacing:.01em}p{margin:0;color:#8b93a7;font-size:13px}</style></head><body><div class="card"><div class="mark"><svg viewBox="0 0 36 34" fill="#0a0e14" xmlns="http://www.w3.org/2000/svg"><path fill-rule="evenodd" clip-rule="evenodd" d="M9.13883 7.06589V0.164429L13.0938 0.164429V6.175L14.5178 7.4346C15.577 6.68656 16.7337 6.27495 17.945 6.27495C19.1731 6.27495 20.3451 6.69807 21.4163 7.46593L22.8757 6.175V0.164429L26.8307 0.164429V7.06589V7.95679L26.1634 8.54706L24.0775 10.3922C24.3436 10.8108 24.5958 11.2563 24.8327 11.7262L26.0467 11.4215L28.6971 8.08749L31.793 10.5487L28.7257 14.407L28.3089 14.9313L27.6592 15.0944L26.2418 15.4502C26.3124 15.7082 26.3793 15.9701 26.4422 16.2355L28.653 16.6566L29.092 16.7402L29.4524 17.0045L35.3849 21.355L33.0461 24.5444L27.474 20.4581L27.0719 20.3816C27.1214 21.0613 27.147 21.7543 27.147 22.4577C27.147 22.5398 27.1466 22.6214 27.1459 22.7024L29.5889 23.7911L30.3219 24.1177L30.62 24.8629L33.6873 32.5312L30.0152 34L27.246 27.0769L26.7298 26.8469C25.5612 32.2432 22.0701 33.8808 17.945 33.8808C13.8382 33.8808 10.3598 32.2577 9.17593 26.9185L8.82034 27.0769L6.05109 34L2.37897 32.5312L5.44629 24.8629L5.74435 24.1177L6.47743 23.7911L8.74487 22.7806C8.74366 22.6739 8.74305 22.5663 8.74305 22.4577C8.74305 21.7616 8.76804 21.0758 8.81654 20.4028L8.52606 20.4581L2.95395 24.5444L0.615112 21.355L6.54761 17.0045L6.908 16.7402L7.34701 16.6566L9.44264 16.2575C9.50917 15.9756 9.5801 15.6978 9.65528 15.4242L8.34123 15.0944L7.69155 14.9313L7.27471 14.407L4.20739 10.5487L7.30328 8.08749L9.95376 11.4215L11.0697 11.7016C11.3115 11.2239 11.5692 10.7716 11.8412 10.3473L9.80612 8.54706L9.13883 7.95679V7.06589Z"/></svg></div><h1>Signed in to Spider Cloud</h1><p>You can close this tab and return to your terminal.</p></div></body></html>"##;

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

        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{CALLBACK_PAGE}",
                    CALLBACK_PAGE.len()
                )
                .as_bytes(),
            )
            .await;

        if !target.starts_with("/callback") {
            continue;
        }
        let params = query_pairs(&target);
        let get = |key: &str| {
            params
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        };

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
    Ok(format!(
        "{scheme}://{}",
        rest.split('/').next().unwrap_or(rest)
    ))
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
