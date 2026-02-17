use anyhow::{Context, Result};
use ethers::providers::{Http, Provider, Ws};
use std::time::Duration;

pub async fn connect_ws_with_timeout(url: &str, timeout: Duration) -> Result<Provider<Ws>> {
    let connect = tokio::time::timeout(timeout, Provider::<Ws>::connect(url))
        .await
        .with_context(|| format!("WebSocket connect timed out after {}s", timeout.as_secs()))?;
    connect.with_context(|| format!("WebSocket connect failed for {}", masked_rpc_url(url)))
}

pub fn http_provider(url: &str) -> Result<Provider<Http>> {
    Provider::<Http>::try_from(url)
        .with_context(|| format!("Failed to initialize HTTP provider for {}", masked_rpc_url(url)))
}

pub fn reconnect_backoff(initial_ms: u64, max_ms: u64, attempt: u32) -> Duration {
    let step = attempt.saturating_sub(1).min(10);
    let factor = 1_u64 << step;
    let max_ms = max_ms.max(initial_ms);
    let delay_ms = initial_ms.saturating_mul(factor).min(max_ms);
    Duration::from_millis(delay_ms)
}

pub fn masked_rpc_url(url: &str) -> String {
    if let Ok(parsed) = reqwest::Url::parse(url) {
        let host = parsed.host_str().unwrap_or("<unknown-host>");
        let scheme = parsed.scheme();
        let path = parsed.path().trim_matches('/');
        return if path.is_empty() {
            format!("{scheme}://{host}/<hidden>")
        } else {
            format!("{scheme}://{host}/.../<hidden>")
        };
    }
    "<invalid-url>".to_string()
}
