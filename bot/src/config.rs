use crate::types::market::{Market, MarketKind};
use anyhow::{Context, Result};
use dotenvy::from_filename_override;
use ethers::types::Address;
use std::env;
use std::str::FromStr;

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub network_name: String,
    pub rpc_https_url: String,
    pub rpc_wss_url: String,
    pub expected_chain_id: u64,
    pub ws_connect_timeout_secs: u64,
    pub http_poll_interval_secs: u64,
    pub ws_reconnect_initial_ms: u64,
    pub ws_reconnect_max_ms: u64,
}

#[derive(Clone, Debug)]
pub struct PoolListenerConfig {
    pub v2_market: Market,
    pub v3_market: Market,
}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self> {
        load_env_file();
        Ok(Self {
            network_name: env_or_default("NETWORK_NAME", "base-mainnet"),
            rpc_https_url: env_required("BASE_RPC_HTTPS_URL")?,
            rpc_wss_url: env_required("BASE_RPC_WSS_URL")?,
            expected_chain_id: env_parse_or_default("CHAIN_ID", 8453_u64)?,
            ws_connect_timeout_secs: env_parse_or_default("WS_CONNECT_TIMEOUT_SECS", 15_u64)?,
            http_poll_interval_secs: env_parse_or_default("HTTP_POLL_INTERVAL_SECS", 2_u64)?,
            ws_reconnect_initial_ms: env_parse_or_default("WS_RECONNECT_INITIAL_MS", 1_000_u64)?,
            ws_reconnect_max_ms: env_parse_or_default("WS_RECONNECT_MAX_MS", 30_000_u64)?,
        })
    }
}

impl PoolListenerConfig {
    pub fn from_env() -> Result<Self> {
        load_env_file();

        let v2_market = Market::new(
            MarketKind::V2Sync,
            parse_address("BASE_V2_POOL")?,
            parse_address("BASE_V2_TOKEN0")?,
            parse_address("BASE_V2_TOKEN1")?,
            env_or_default("BASE_V2_TOKEN0_SYMBOL", "TOKEN0"),
            env_or_default("BASE_V2_TOKEN1_SYMBOL", "TOKEN1"),
            env_parse_or_default("BASE_V2_TOKEN0_DECIMALS", 18_u8)?,
            env_parse_or_default("BASE_V2_TOKEN1_DECIMALS", 18_u8)?,
        );

        let v3_market = Market::new(
            MarketKind::V3Swap,
            parse_address("BASE_V3_POOL")?,
            parse_address("BASE_V3_TOKEN0")?,
            parse_address("BASE_V3_TOKEN1")?,
            env_or_default("BASE_V3_TOKEN0_SYMBOL", "TOKEN0"),
            env_or_default("BASE_V3_TOKEN1_SYMBOL", "TOKEN1"),
            env_parse_or_default("BASE_V3_TOKEN0_DECIMALS", 18_u8)?,
            env_parse_or_default("BASE_V3_TOKEN1_DECIMALS", 18_u8)?,
        );

        Ok(Self {
            v2_market,
            v3_market,
        })
    }
}

fn load_env_file() {
    from_filename_override(".env").ok();
}

fn env_required(key: &str) -> Result<String> {
    let raw = env::var(key).with_context(|| format!("{key} is not set. Add it to your .env file."))?;
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'').to_string();
    if trimmed.is_empty() {
        anyhow::bail!("{key} is empty in .env");
    }
    Ok(trimmed)
}

fn env_or_default(key: &str, default: &str) -> String {
    env::var(key)
        .map(|value| value.trim().trim_matches('"').trim_matches('\'').to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_parse_or_default<T>(key: &str, default: T) -> Result<T>
where
    T: FromStr + Copy,
    T::Err: std::fmt::Display,
{
    match env::var(key) {
        Ok(raw) => {
            let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
            if trimmed.is_empty() {
                return Ok(default);
            }
            trimmed
                .parse::<T>()
                .map_err(|err| anyhow::anyhow!("Failed parsing {key}: {err}"))
        }
        Err(_) => Ok(default),
    }
}

fn parse_address(key: &str) -> Result<Address> {
    let raw = env_required(key)?;
    Address::from_str(&raw).with_context(|| format!("Invalid address in {key}: {raw}"))
}
