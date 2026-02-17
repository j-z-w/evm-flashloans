use anyhow::{Context, Result};
use dotenvy::from_filename_override;
use ethers::providers::{Middleware, Provider, Ws};
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{Value, json};
use std::env;
use std::time::{Duration, Instant};
use tokio::time::sleep;

enum WsLoopExit {
    Shutdown,
    Disconnected,
}

struct ErrorLogGate {
    min_interval: Duration,
    last_emit: Option<Instant>,
    suppressed: u64,
}

impl ErrorLogGate {
    fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_emit: None,
            suppressed: 0,
        }
    }

    fn log(&mut self, prefix: &str, details: &str) {
        let now = Instant::now();
        let should_emit = self
            .last_emit
            .is_none_or(|last| now.duration_since(last) >= self.min_interval);

        if should_emit {
            if self.suppressed > 0 {
                eprintln!(
                    "{prefix}: {details} (suppressed {} similar log lines)",
                    self.suppressed
                );
                self.suppressed = 0;
            } else {
                eprintln!("{prefix}: {details}");
            }
            self.last_emit = Some(now);
        } else {
            self.suppressed = self.suppressed.saturating_add(1);
        }
    }

    fn flush(&mut self, prefix: &str) {
        if self.suppressed > 0 {
            eprintln!("{prefix}: suppressed {} similar log lines", self.suppressed);
            self.suppressed = 0;
        }
    }
}

fn env_url(key: &str) -> Result<String> {
    let raw = env::var(key).with_context(|| format!("{key} is not set. Add it to your .env file."))?;
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'').to_string();
    if trimmed.is_empty() {
        anyhow::bail!("{key} is empty in .env");
    }
    Ok(trimmed)
}

fn env_u64_or_default(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .map(|value| value.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn masked_rpc_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) => {
            let scheme = parsed.scheme();
            let host = parsed.host_str().unwrap_or("<unknown-host>");
            format!("{scheme}://{host}/.../<hidden>")
        }
        Err(_) => "<invalid-url>".to_string(),
    }
}

fn sanitize_token(token: &str) -> String {
    let leading_bytes = token
        .chars()
        .take_while(|c| matches!(c, '"' | '\'' | '(' | '[' | '{' | '<'))
        .map(char::len_utf8)
        .sum::<usize>();
    let trailing_bytes = token
        .chars()
        .rev()
        .take_while(|c| matches!(c, '"' | '\'' | ')' | ']' | '}' | '>' | ',' | '.' | ';' | ':'))
        .map(char::len_utf8)
        .sum::<usize>();

    if leading_bytes + trailing_bytes >= token.len() {
        return token.to_string();
    }

    let core_end = token.len().saturating_sub(trailing_bytes);
    let core = &token[leading_bytes..core_end];
    if core.starts_with("https://")
        || core.starts_with("http://")
        || core.starts_with("wss://")
        || core.starts_with("ws://")
    {
        let mut masked = String::new();
        masked.push_str(&token[..leading_bytes]);
        masked.push_str(&masked_rpc_url(core));
        masked.push_str(&token[core_end..]);
        return masked;
    }

    token.to_string()
}

fn sanitize_log_text(message: &str) -> String {
    message
        .split_whitespace()
        .map(sanitize_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn sanitize_error(err: impl std::fmt::Display) -> String {
    sanitize_log_text(&err.to_string())
}

fn reconnect_backoff(initial_ms: u64, max_ms: u64, attempt: u32) -> Duration {
    let step = attempt.saturating_sub(1).min(10);
    let factor = 1_u64 << step;
    let cap = max_ms.max(initial_ms);
    Duration::from_millis(initial_ms.saturating_mul(factor).min(cap))
}

async fn wait_or_shutdown(duration: Duration) -> bool {
    tokio::select! {
        _ = sleep(duration) => false,
        _ = tokio::signal::ctrl_c() => true,
    }
}

fn parse_hex_u64(value: &str) -> Result<u64> {
    u64::from_str_radix(value.trim_start_matches("0x"), 16)
        .with_context(|| format!("Invalid hex value: {value}"))
}

fn print_block_if_new(last_block: &mut Option<u64>, block: u64) {
    if *last_block != Some(block) {
        println!("New Block: {block}");
        *last_block = Some(block);
    }
}

async fn fetch_chain_id_http(client: &reqwest::Client, https_url: &str) -> Result<u64> {
    let payload = json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_chainId",
        "params": []
    });

    let response = client
        .post(https_url)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await
        .context("Failed eth_chainId request over HTTPS")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read eth_chainId response body")?;
    if !status.is_success() {
        anyhow::bail!("eth_chainId returned {status}: {body}");
    }

    let value: Value = serde_json::from_str(&body)
        .with_context(|| format!("Invalid eth_chainId JSON response: {body}"))?;
    let result = value
        .get("result")
        .and_then(Value::as_str)
        .context("eth_chainId response missing string result")?;
    parse_hex_u64(result)
}

async fn fetch_block_number_http(client: &reqwest::Client, https_url: &str) -> Result<u64> {
    let payload = json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_blockNumber",
        "params": []
    });

    let response = client
        .post(https_url)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await
        .context("Failed eth_blockNumber request over HTTPS")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read eth_blockNumber response body")?;
    if !status.is_success() {
        anyhow::bail!("eth_blockNumber returned {status}: {body}");
    }

    let value: Value = serde_json::from_str(&body)
        .with_context(|| format!("Invalid eth_blockNumber JSON response: {body}"))?;
    let result = value
        .get("result")
        .and_then(Value::as_str)
        .context("eth_blockNumber response missing string result")?;
    parse_hex_u64(result)
}

async fn log_http_chain_id(client: &reqwest::Client, https_url: &str, expected_chain_id: u64) {
    match fetch_chain_id_http(client, https_url).await {
        Ok(actual) => {
            if actual == expected_chain_id {
                println!("Mode: http-fallback, chain_id={actual}");
            } else {
                eprintln!("Mode: http-fallback, chain_id={actual}, expected_chain_id={expected_chain_id}");
            }
        }
        Err(err) => eprintln!("HTTP chain id diagnostic failed: {}", sanitize_error(&err)),
    }
}

async fn run_http_polling_window(
    client: &reqwest::Client,
    https_url: &str,
    expected_chain_id: u64,
    poll_interval: Duration,
    window: Duration,
    last_block: &mut Option<u64>,
) -> bool {
    log_http_chain_id(client, https_url, expected_chain_id).await;
    let mut error_gate = ErrorLogGate::new(Duration::from_secs(15));

    let started = Instant::now();
    loop {
        if started.elapsed() >= window {
            error_gate.flush("HTTPS polling errors");
            return false;
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => return true,
            result = fetch_block_number_http(client, https_url) => {
                match result {
                    Ok(block) => print_block_if_new(last_block, block),
                    Err(err) => {
                        error_gate.log("HTTPS polling error (retrying)", &sanitize_error(&err))
                    }
                }
            }
        }

        let remaining = window.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return false;
        }
        let sleep_for = poll_interval.min(remaining);
        if wait_or_shutdown(sleep_for).await {
            error_gate.flush("HTTPS polling errors");
            return true;
        }
    }
}

async fn connect_ws(url: &str, timeout: Duration) -> Result<Provider<Ws>> {
    let connect = tokio::time::timeout(timeout, Provider::<Ws>::connect(url))
        .await
        .with_context(|| format!("WebSocket connect timed out after {}s", timeout.as_secs()))?;
    connect.with_context(|| format!("WebSocket connect failed for {}", masked_rpc_url(url)))
}

async fn run_ws_loop(provider: Provider<Ws>, expected_chain_id: u64, last_block: &mut Option<u64>) -> WsLoopExit {
    match provider.get_chainid().await {
        Ok(actual) => {
            let actual = actual.as_u64();
            if actual == expected_chain_id {
                println!("Mode: ws, chain_id={actual}");
            } else {
                eprintln!("Mode: ws, chain_id={actual}, expected_chain_id={expected_chain_id}");
            }
        }
        Err(err) => eprintln!("WS chain id diagnostic failed: {}", sanitize_error(&err)),
    }

    let mut blocks = match provider.subscribe_blocks().await {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("WS subscribe failed: {}", sanitize_error(&err));
            return WsLoopExit::Disconnected;
        }
    };

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return WsLoopExit::Shutdown,
            maybe_block = blocks.next() => {
                match maybe_block {
                    Some(block) => {
                        match block.number {
                            Some(number) => print_block_if_new(last_block, number.as_u64()),
                            None => println!("New Block: <pending>"),
                        }
                    }
                    None => {
                        eprintln!("WebSocket block stream ended.");
                        return WsLoopExit::Disconnected;
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    from_filename_override(".env").ok();

    let wss_url = env_url("BASE_RPC_WSS_URL")?;
    let https_url = env_url("BASE_RPC_HTTPS_URL")?;
    let expected_chain_id = env_u64_or_default("CHAIN_ID", 8453);
    let ws_connect_timeout = Duration::from_secs(env_u64_or_default("WS_CONNECT_TIMEOUT_SECS", 15));
    let ws_backoff_initial_ms = env_u64_or_default("WS_RECONNECT_INITIAL_MS", 1_000);
    let ws_backoff_max_ms = env_u64_or_default("WS_RECONNECT_MAX_MS", 30_000);
    let http_poll_interval = Duration::from_secs(env_u64_or_default("HTTP_POLL_INTERVAL_SECS", 2));

    println!(
        "Startup Diagnostics: ws_provider={}, http_provider={}, expected_chain_id={}, ws_timeout_s={}, http_poll_s={}, mode=ws-first",
        masked_rpc_url(&wss_url),
        masked_rpc_url(&https_url),
        expected_chain_id,
        ws_connect_timeout.as_secs(),
        http_poll_interval.as_secs()
    );

    let client = reqwest::Client::builder()
        .build()
        .context("Failed to initialize HTTP client")?;
    let mut last_block: Option<u64> = None;
    let mut ws_attempt: u32 = 0;

    loop {
        match connect_ws(&wss_url, ws_connect_timeout).await {
            Ok(provider) => {
                ws_attempt = 0;
                println!("Connected via WebSocket.");

                match run_ws_loop(provider, expected_chain_id, &mut last_block).await {
                    WsLoopExit::Shutdown => break,
                    WsLoopExit::Disconnected => {
                        ws_attempt = ws_attempt.saturating_add(1);
                        let wait = reconnect_backoff(ws_backoff_initial_ms, ws_backoff_max_ms, ws_attempt);
                        eprintln!("WS disconnected. Reconnecting in {} ms.", wait.as_millis());
                        if wait_or_shutdown(wait).await {
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                ws_attempt = ws_attempt.saturating_add(1);
                let wait = reconnect_backoff(ws_backoff_initial_ms, ws_backoff_max_ms, ws_attempt);
                eprintln!("WS connect failed: {}", sanitize_error(&err));
                eprintln!("Falling back to HTTPS polling for {} ms.", wait.as_millis());
                if run_http_polling_window(
                    &client,
                    &https_url,
                    expected_chain_id,
                    http_poll_interval,
                    wait,
                    &mut last_block,
                )
                .await
                {
                    break;
                }
            }
        }
    }

    println!("Heartbeat shutdown complete.");
    Ok(())
}
