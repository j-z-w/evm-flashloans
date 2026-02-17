use anyhow::{Context, Result};
use dotenvy::from_filename_override;
use ethers::providers::{Middleware, Provider, Ws};
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{json, Value};
use std::env;
use std::time::Duration;
use tokio::time::sleep;

fn env_url(key: &str) -> Result<String> {
    let raw = env::var(key).with_context(|| format!("{key} is not set. Add it to your .env file."))?;
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'').to_string();
    if trimmed.is_empty() {
        anyhow::bail!("{key} is empty in .env");
    }
    Ok(trimmed)
}

#[tokio::main]
async fn main() -> Result<()> {
    from_filename_override(".env").ok();

    let wss_url = env_url("BASE_RPC_WSS_URL")?;
    let ws_connect = tokio::time::timeout(Duration::from_secs(15), Provider::<Ws>::connect(&wss_url)).await;

    match ws_connect {
        Ok(Ok(provider)) => {
            println!("Connected via WebSocket.");
            let mut blocks = provider
                .subscribe_blocks()
                .await
                .context("Connected, but failed to subscribe to new blocks")?;

            while let Some(block) = blocks.next().await {
                match block.number {
                    Some(number) => println!("New Block: {number}"),
                    None => println!("New Block: <pending>"),
                }
            }
        }
        Ok(Err(err)) => {
            println!("WebSocket connect failed: {err}");
            println!("Falling back to HTTPS polling.");
            run_https_polling().await?;
        }
        Err(_) => {
            println!("WebSocket connect timed out after 15 seconds.");
            println!("Falling back to HTTPS polling.");
            run_https_polling().await?;
        }
    }

    Ok(())
}

async fn run_https_polling() -> Result<()> {
    let https_url = env_url("BASE_RPC_HTTPS_URL")?;
    let client = reqwest::Client::builder()
        .build()
        .context("Failed to initialize HTTP client")?;

    let mut last = None;
    loop {
        let payload = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": []
        });

        let response = client
            .post(&https_url)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await
            .context("Failed to fetch block number over HTTPS")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read HTTPS response body")?;
        if !status.is_success() {
            anyhow::bail!("HTTPS RPC returned {status}: {body}");
        }

        let value: Value = serde_json::from_str(&body)
            .with_context(|| format!("Invalid JSON-RPC response: {body}"))?;
        let result = value
            .get("result")
            .and_then(Value::as_str)
            .context("JSON-RPC response missing string result field")?;
        let block = u64::from_str_radix(result.trim_start_matches("0x"), 16)
            .with_context(|| format!("Invalid block number hex result: {result}"))?;

        if last != Some(block.to_string()) {
            println!("New Block: {block}");
            last = Some(block.to_string());
        }

        sleep(Duration::from_secs(2)).await;
    }
}
