use anyhow::{Context, Result};
use dotenvy::from_filename_override;
use ethers::abi::{ParamType, Token, decode, encode};
use ethers::providers::{Http, Middleware, Provider};
use ethers::types::transaction::eip2718::TypedTransaction;
use ethers::types::{Address, BlockId, BlockNumber, Bytes, TransactionRequest, U256};
use ethers::utils::id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
struct ShadowConfig {
    network: String,
    chain_id: u64,
    poll_interval_ms: u64,
    max_block_age_secs: u64,
    route: RouteConfig,
    input_sizes_wei: Vec<String>,
    flash_loan_fee_bps: u64,
    gas_units_estimate: u64,
    max_gas_price_wei: String,
    min_profit_wei: String,
}

#[derive(Debug, Deserialize)]
struct RouteConfig {
    name: String,
    token_in_symbol: String,
    token_mid_symbol: String,
    token_out_symbol: String,
    token_in_address: String,
    token_mid_address: String,
    token_in_decimals: u8,
    token_mid_decimals: u8,
    v2_pair: String,
    v2_fee_bps: u64,
    v3_pool: String,
    v3_pool_fee: u32,
    v3_quoter_v2: String,
}

#[derive(Debug, Serialize)]
struct ShadowDecisionLog {
    run_id: String,
    ts_unix_ms: u64,
    network: String,
    route: String,
    block: u64,
    block_age_secs: u64,
    input_wei: String,
    gas_price_wei: String,
    gas_cost_wei: String,
    flash_fee_wei: String,
    total_cost_wei: String,
    v2_out_mid_wei: String,
    v3_out_wei: String,
    net_wei: String,
    edge_bps: String,
    v3_quote_latency_ms: u64,
    decision: String,
    reason: String,
}

#[derive(Debug)]
struct ParsedRoute {
    name: String,
    token_in: Address,
    token_mid: Address,
    v2_pair: Address,
    v2_fee_bps: u64,
    v2_token0_to1: bool,
    v3_pool: Address,
    v3_pool_fee: u32,
    v3_quoter_v2: Address,
}

struct EmitContext<'a> {
    run_id: &'a str,
    network: &'a str,
    route: &'a str,
    block: u64,
    block_age_secs: u64,
    input: U256,
    gas_price: U256,
    gas_cost: U256,
    flash_fee: U256,
    v2_out_mid: U256,
    v3_out: U256,
    v3_quote_latency_ms: u64,
}

struct ErrorEmitContext<'a> {
    run_id: &'a str,
    network: &'a str,
    route: &'a str,
    block: u64,
    block_age_secs: u64,
    input_sizes: &'a [U256],
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

#[derive(Default)]
struct ShadowStats {
    blocks_seen: u64,
    rows_emitted: u64,
    would_trade: u64,
    would_skip: u64,
    reason_counts: BTreeMap<String, u64>,
}

impl ShadowStats {
    fn record(&mut self, decision: &str, reason: &str) {
        self.rows_emitted = self.rows_emitted.saturating_add(1);
        if decision == "would_trade" {
            self.would_trade = self.would_trade.saturating_add(1);
        } else {
            self.would_skip = self.would_skip.saturating_add(1);
        }

        let key = normalized_reason(reason);
        let count = self.reason_counts.entry(key).or_insert(0);
        *count = count.saturating_add(1);
    }
}

#[derive(Serialize)]
struct ShadowSummaryLog {
    run_id: String,
    network: String,
    route: String,
    summary_kind: String,
    latest_block: u64,
    blocks_seen: u64,
    rows_emitted: u64,
    would_trade: u64,
    would_skip: u64,
    top_reasons: Vec<ReasonCount>,
}

#[derive(Serialize)]
struct ReasonCount {
    reason: String,
    count: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    from_filename_override(".env").ok();

    let config_path = env::var("ROUTES_CONFIG_PATH")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "bot/config/routes.base.json".to_string());

    let config = load_config(&config_path)?;
    let provider = http_provider_from_env()?;
    validate_network(&provider, config.chain_id).await?;

    let max_gas_price = parse_u256_dec(&config.max_gas_price_wei)?;
    let min_profit = parse_u256_dec(&config.min_profit_wei)?;
    let input_sizes = parse_u256_list(&config.input_sizes_wei)?;
    let route = parse_and_validate_route(&provider, &config.route).await?;

    let max_blocks = env::var("SHADOW_MAX_BLOCKS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok());
    let summary_every_blocks = env_u64_or_default("SHADOW_SUMMARY_EVERY_BLOCKS", 25).max(1);
    let verbose_block_logs = env_bool_or_default("SHADOW_VERBOSE_BLOCK_LOGS", false);
    let run_id = format!("shadow-{}", unix_now_millis()?);
    let mut stats = ShadowStats::default();
    let mut infra_error_gate = ErrorLogGate::new(Duration::from_secs(15));

    eprintln!(
        "Shadow mode start: run_id={}, network={}, route={}, leg=v2->v3, pair={:#x}, pool={:#x}, quoter={:#x}, inputs={}, polling_ms={}, max_blocks={}, summary_every_blocks={}, verbose_block_logs={}",
        run_id,
        config.network,
        route.name,
        route.v2_pair,
        route.v3_pool,
        route.v3_quoter_v2,
        input_sizes.len(),
        config.poll_interval_ms,
        max_blocks.unwrap_or(0),
        summary_every_blocks,
        verbose_block_logs
    );

    let mut last_block: Option<u64> = None;
    let mut processed_blocks: u64 = 0;
    let poll_interval = Duration::from_millis(config.poll_interval_ms.max(250));

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("Shadow mode stopped.");
                break;
            }
            _ = tokio::time::sleep(poll_interval) => {}
        }

        let block_number = match provider.get_block_number().await {
            Ok(value) => value.as_u64(),
            Err(err) => {
                infra_error_gate.log("block fetch failed (retrying)", &sanitize_error(&err));
                continue;
            }
        };
        if last_block == Some(block_number) {
            continue;
        }
        last_block = Some(block_number);
        processed_blocks = processed_blocks.saturating_add(1);
        stats.blocks_seen = stats.blocks_seen.saturating_add(1);

        let block_timestamp = match provider.get_block(block_number).await {
            Ok(Some(block)) => block.timestamp.as_u64(),
            Ok(None) => {
                log_route_error(
                    ErrorEmitContext {
                        run_id: &run_id,
                        network: &config.network,
                        route: &route.name,
                        block: block_number,
                        block_age_secs: 0,
                        input_sizes: &input_sizes,
                    },
                    "quote_error",
                    "missing_block".to_string(),
                    &mut stats,
                );
                continue;
            }
            Err(err) => {
                infra_error_gate.log("block payload fetch failed", &sanitize_error(&err));
                log_route_error(
                    ErrorEmitContext {
                        run_id: &run_id,
                        network: &config.network,
                        route: &route.name,
                        block: block_number,
                        block_age_secs: 0,
                        input_sizes: &input_sizes,
                    },
                    "quote_error",
                    "block_fetch_failed".to_string(),
                    &mut stats,
                );
                continue;
            }
        };

        let now = unix_now_secs()?;
        let block_age_secs = now.saturating_sub(block_timestamp);
        if block_age_secs > config.max_block_age_secs {
            log_route_error(
                ErrorEmitContext {
                    run_id: &run_id,
                    network: &config.network,
                    route: &route.name,
                    block: block_number,
                    block_age_secs,
                    input_sizes: &input_sizes,
                },
                "stale_data",
                format!(
                    "block_age_secs={}",
                    block_age_secs
                ),
                &mut stats,
            );
            continue;
        }

        let call_block = Some(BlockId::Number(BlockNumber::Number(block_number.into())));

        let gas_price = match provider.get_gas_price().await {
            Ok(value) => value,
            Err(err) => {
                infra_error_gate.log("gas price fetch failed", &sanitize_error(&err));
                log_route_error(
                    ErrorEmitContext {
                        run_id: &run_id,
                        network: &config.network,
                        route: &route.name,
                        block: block_number,
                        block_age_secs,
                        input_sizes: &input_sizes,
                    },
                    "quote_error",
                    "gas_price_failed".to_string(),
                    &mut stats,
                );
                continue;
            }
        };

        let (reserve0, reserve1) = match get_v2_reserves(&provider, route.v2_pair, call_block).await {
            Ok(values) => values,
            Err(err) => {
                infra_error_gate.log("v2 reserves fetch failed", &sanitize_error(&err));
                log_route_error(
                    ErrorEmitContext {
                        run_id: &run_id,
                        network: &config.network,
                        route: &route.name,
                        block: block_number,
                        block_age_secs,
                        input_sizes: &input_sizes,
                    },
                    "quote_error",
                    "v2_reserves_failed".to_string(),
                    &mut stats,
                );
                continue;
            }
        };
        if reserve0.is_zero() || reserve1.is_zero() {
            log_route_error(
                ErrorEmitContext {
                    run_id: &run_id,
                    network: &config.network,
                    route: &route.name,
                    block: block_number,
                    block_age_secs,
                    input_sizes: &input_sizes,
                },
                "bad_pool_state",
                "v2_zero_reserve".to_string(),
                &mut stats,
            );
            continue;
        }

        if verbose_block_logs {
            eprintln!(
                "Block diagnostics: run_id={}, block={}, block_age_secs={}, gas_price_wei={}, reserve0={}, reserve1={}",
                run_id, block_number, block_age_secs, gas_price, reserve0, reserve1
            );
        }

        for input in &input_sizes {
            let gas_cost = gas_price.saturating_mul(U256::from(config.gas_units_estimate));
            let flash_fee = fee_from_bps(*input, config.flash_loan_fee_bps);
            let v2_out_mid = quote_v2_exact_in(
                *input,
                reserve0,
                reserve1,
                route.v2_fee_bps,
                route.v2_token0_to1,
            );
            if v2_out_mid.is_zero() {
                emit_row(
                    EmitContext {
                        run_id: &run_id,
                        network: &config.network,
                        route: &route.name,
                        block: block_number,
                        block_age_secs,
                        input: *input,
                        gas_price,
                        gas_cost,
                        flash_fee,
                        v2_out_mid: U256::zero(),
                        v3_out: U256::zero(),
                        v3_quote_latency_ms: 0,
                    },
                    "would_skip",
                    "bad_pool_state:v2_out_zero",
                    &mut stats,
                )?;
                continue;
            }

            let v3_quote_started = Instant::now();
            let v3_out = match quote_v3_exact_input_single(
                &provider,
                route.v3_quoter_v2,
                route.token_mid,
                route.token_in,
                v2_out_mid,
                route.v3_pool_fee,
                call_block,
            )
            .await
            {
                Ok(value) => value,
                Err(err) => {
                    infra_error_gate.log("v3 quoter call failed", &sanitize_error(&err));
                    emit_row(
                        EmitContext {
                            run_id: &run_id,
                            network: &config.network,
                            route: &route.name,
                            block: block_number,
                            block_age_secs,
                            input: *input,
                            gas_price,
                            gas_cost,
                            flash_fee,
                            v2_out_mid,
                            v3_out: U256::zero(),
                            v3_quote_latency_ms: v3_quote_started.elapsed().as_millis() as u64,
                        },
                        "would_skip",
                        "quote_error:v3_quoter_failed",
                        &mut stats,
                    )?;
                    continue;
                }
            };
            let v3_quote_latency_ms = v3_quote_started.elapsed().as_millis() as u64;

            if gas_price > max_gas_price {
                emit_row(
                    EmitContext {
                        run_id: &run_id,
                        network: &config.network,
                        route: &route.name,
                        block: block_number,
                        block_age_secs,
                        input: *input,
                        gas_price,
                        gas_cost,
                        flash_fee,
                        v2_out_mid,
                        v3_out,
                        v3_quote_latency_ms,
                    },
                    "would_skip",
                    "gas_too_high",
                    &mut stats,
                )?;
                continue;
            }

            let total_cost = input.saturating_add(flash_fee).saturating_add(gas_cost);
            if v3_out <= total_cost {
                emit_row(
                    EmitContext {
                        run_id: &run_id,
                        network: &config.network,
                        route: &route.name,
                        block: block_number,
                        block_age_secs,
                        input: *input,
                        gas_price,
                        gas_cost,
                        flash_fee,
                        v2_out_mid,
                        v3_out,
                        v3_quote_latency_ms,
                    },
                    "would_skip",
                    "below_min_profit",
                    &mut stats,
                )?;
                continue;
            }

            let net = v3_out - total_cost;
            if net < min_profit {
                emit_row(
                    EmitContext {
                        run_id: &run_id,
                        network: &config.network,
                        route: &route.name,
                        block: block_number,
                        block_age_secs,
                        input: *input,
                        gas_price,
                        gas_cost,
                        flash_fee,
                        v2_out_mid,
                        v3_out,
                        v3_quote_latency_ms,
                    },
                    "would_skip",
                    "below_min_profit",
                    &mut stats,
                )?;
                continue;
            }

            emit_row(
                EmitContext {
                    run_id: &run_id,
                    network: &config.network,
                    route: &route.name,
                    block: block_number,
                    block_age_secs,
                    input: *input,
                    gas_price,
                    gas_cost,
                    flash_fee,
                    v2_out_mid,
                    v3_out,
                    v3_quote_latency_ms,
                },
                "would_trade",
                "edge_above_threshold",
                &mut stats,
            )?;
        }

        if processed_blocks.is_multiple_of(summary_every_blocks) {
            emit_summary(
                &run_id,
                &config.network,
                &route.name,
                block_number,
                "periodic",
                &stats,
            );
        }

        if max_blocks.is_some_and(|limit| processed_blocks >= limit) {
            eprintln!("Shadow mode reached SHADOW_MAX_BLOCKS={processed_blocks}; exiting.");
            break;
        }
    }

    infra_error_gate.flush("shadow infra errors");

    let latest_block = last_block.unwrap_or(0);
    emit_summary(
        &run_id,
        &config.network,
        &route.name,
        latest_block,
        "final",
        &stats,
    );

    Ok(())
}

fn load_config(path: &str) -> Result<ShadowConfig> {
    let content = fs::read_to_string(path).with_context(|| format!("failed reading config at {path}"))?;
    serde_json::from_str(&content).with_context(|| format!("failed parsing JSON config at {path}"))
}

fn http_provider_from_env() -> Result<Provider<Http>> {
    let raw = env::var("BASE_RPC_HTTPS_URL")
        .with_context(|| "BASE_RPC_HTTPS_URL is not set. Add it to .env or your shell env.")?;
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
    Provider::<Http>::try_from(trimmed)
        .with_context(|| "failed to initialize HTTP provider from BASE_RPC_HTTPS_URL")
}

async fn validate_network(provider: &Provider<Http>, expected_chain_id: u64) -> Result<()> {
    let actual = provider
        .get_chainid()
        .await
        .context("failed to fetch chain id from RPC")?
        .as_u64();
    if actual != expected_chain_id {
        anyhow::bail!("chain id mismatch: expected {expected_chain_id}, got {actual}");
    }
    Ok(())
}

async fn parse_and_validate_route(provider: &Provider<Http>, raw: &RouteConfig) -> Result<ParsedRoute> {
    let token_in = parse_address(&raw.token_in_address)?;
    let token_mid = parse_address(&raw.token_mid_address)?;
    let v2_pair = parse_address(&raw.v2_pair)?;
    let v3_pool = parse_address(&raw.v3_pool)?;
    let v3_quoter_v2 = parse_address(&raw.v3_quoter_v2)?;

    let v2_token0 = get_address_view(provider, v2_pair, "token0()").await?;
    let v2_token1 = get_address_view(provider, v2_pair, "token1()").await?;
    let v2_token0_to1 = if v2_token0 == token_in && v2_token1 == token_mid {
        true
    } else if v2_token0 == token_mid && v2_token1 == token_in {
        false
    } else {
        anyhow::bail!(
            "bad_pool_state: v2 pair token mismatch pair={:#x} token0={:#x} token1={:#x}",
            v2_pair,
            v2_token0,
            v2_token1
        );
    };

    let v3_token0 = get_address_view(provider, v3_pool, "token0()").await?;
    let v3_token1 = get_address_view(provider, v3_pool, "token1()").await?;
    let v3_pool_fee = get_u24_view(provider, v3_pool, "fee()").await?;

    let v3_has_tokens =
        (v3_token0 == token_in && v3_token1 == token_mid) || (v3_token0 == token_mid && v3_token1 == token_in);
    if !v3_has_tokens {
        anyhow::bail!(
            "bad_pool_state: v3 pool token mismatch pool={:#x} token0={:#x} token1={:#x}",
            v3_pool,
            v3_token0,
            v3_token1
        );
    }
    if v3_pool_fee != raw.v3_pool_fee {
        anyhow::bail!(
            "bad_pool_state: v3 pool fee mismatch pool={:#x} configured={} onchain={}",
            v3_pool,
            raw.v3_pool_fee,
            v3_pool_fee
        );
    }

    let _ = (
        &raw.token_in_symbol,
        &raw.token_mid_symbol,
        &raw.token_out_symbol,
        raw.token_in_decimals,
        raw.token_mid_decimals,
    );

    Ok(ParsedRoute {
        name: raw.name.clone(),
        token_in,
        token_mid,
        v2_pair,
        v2_fee_bps: raw.v2_fee_bps.min(10_000),
        v2_token0_to1,
        v3_pool,
        v3_pool_fee,
        v3_quoter_v2,
    })
}

fn parse_u256_dec(value: &str) -> Result<U256> {
    U256::from_dec_str(value.trim()).with_context(|| format!("failed parsing decimal U256: {value}"))
}

fn parse_u256_list(values: &[String]) -> Result<Vec<U256>> {
    values.iter().map(|v| parse_u256_dec(v)).collect()
}

fn env_u64_or_default(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .map(|value| value.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_bool_or_default(key: &str, default: bool) -> bool {
    env::var(key)
        .ok()
        .map(|value| value.trim().trim_matches('"').trim_matches('\'').to_ascii_lowercase())
        .and_then(|value| match value.as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
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
        masked.push_str("<redacted-url>");
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

fn fee_from_bps(amount: U256, bps: u64) -> U256 {
    amount
        .saturating_mul(U256::from(bps))
        .checked_div(U256::from(10_000_u64))
        .unwrap_or_else(U256::zero)
}

fn selector(signature: &str) -> [u8; 4] {
    let hash = id(signature);
    [hash[0], hash[1], hash[2], hash[3]]
}

async fn eth_call(provider: &Provider<Http>, to: Address, data: Bytes, block: Option<BlockId>) -> Result<Bytes> {
    let tx: TypedTransaction = TransactionRequest::new().to(to).data(data).into();
    provider
        .call(&tx, block)
        .await
        .with_context(|| format!("eth_call failed on {:#x}", to))
}

async fn get_address_view(provider: &Provider<Http>, contract: Address, signature: &str) -> Result<Address> {
    let out = eth_call(provider, contract, Bytes::from(selector(signature).to_vec()), None).await?;
    let tokens = decode(&[ParamType::Address], out.as_ref())
        .with_context(|| format!("decode failed for {signature} on {:#x}", contract))?;
    match tokens.first() {
        Some(Token::Address(value)) => Ok(*value),
        _ => anyhow::bail!("unexpected address response for {signature} on {:#x}", contract),
    }
}

async fn get_u24_view(provider: &Provider<Http>, contract: Address, signature: &str) -> Result<u32> {
    let out = eth_call(provider, contract, Bytes::from(selector(signature).to_vec()), None).await?;
    let tokens = decode(&[ParamType::Uint(24)], out.as_ref())
        .with_context(|| format!("decode failed for {signature} on {:#x}", contract))?;
    match tokens.first() {
        Some(Token::Uint(value)) => Ok(value.low_u32()),
        _ => anyhow::bail!("unexpected uint24 response for {signature} on {:#x}", contract),
    }
}

async fn get_v2_reserves(provider: &Provider<Http>, pair: Address, block: Option<BlockId>) -> Result<(U256, U256)> {
    let out = eth_call(provider, pair, Bytes::from(selector("getReserves()").to_vec()), block).await?;
    let tokens = decode(
        &[ParamType::Uint(112), ParamType::Uint(112), ParamType::Uint(32)],
        out.as_ref(),
    )
    .context("failed decoding getReserves response")?;
    if tokens.len() != 3 {
        anyhow::bail!("unexpected getReserves token length {}", tokens.len());
    }

    let reserve0 = token_as_uint(&tokens[0])?;
    let reserve1 = token_as_uint(&tokens[1])?;
    Ok((reserve0, reserve1))
}

fn quote_v2_exact_in(
    amount_in: U256,
    reserve0: U256,
    reserve1: U256,
    fee_bps: u64,
    token0_to1: bool,
) -> U256 {
    let (reserve_in, reserve_out) = if token0_to1 {
        (reserve0, reserve1)
    } else {
        (reserve1, reserve0)
    };

    if amount_in.is_zero() || reserve_in.is_zero() || reserve_out.is_zero() {
        return U256::zero();
    }

    let fee_numerator = U256::from(10_000_u64.saturating_sub(fee_bps.min(10_000)));
    let fee_denominator = U256::from(10_000_u64);
    let amount_in_with_fee = amount_in
        .saturating_mul(fee_numerator)
        .checked_div(fee_denominator)
        .unwrap_or_else(U256::zero);

    let numerator = amount_in_with_fee.saturating_mul(reserve_out);
    let denominator = reserve_in.saturating_add(amount_in_with_fee);
    if denominator.is_zero() {
        return U256::zero();
    }
    numerator.checked_div(denominator).unwrap_or_else(U256::zero)
}

async fn quote_v3_exact_input_single(
    provider: &Provider<Http>,
    quoter: Address,
    token_in: Address,
    token_out: Address,
    amount_in: U256,
    fee: u32,
    block: Option<BlockId>,
) -> Result<U256> {
    let mut data = selector("quoteExactInputSingle((address,address,uint256,uint24,uint160))").to_vec();
    let params = Token::Tuple(vec![
        Token::Address(token_in),
        Token::Address(token_out),
        Token::Uint(amount_in),
        Token::Uint(U256::from(fee)),
        Token::Uint(U256::zero()),
    ]);
    data.extend(encode(&[params]));

    let out = eth_call(provider, quoter, Bytes::from(data), block).await?;
    let tokens = decode(
        &[
            ParamType::Uint(256),
            ParamType::Uint(160),
            ParamType::Uint(32),
            ParamType::Uint(256),
        ],
        out.as_ref(),
    )
    .context("failed decoding quoter response")?;
    if tokens.len() != 4 {
        anyhow::bail!("unexpected quoter token length {}", tokens.len());
    }
    token_as_uint(&tokens[0])
}

fn token_as_uint(token: &Token) -> Result<U256> {
    match token {
        Token::Uint(value) => Ok(*value),
        _ => anyhow::bail!("expected uint token, found {token:?}"),
    }
}

fn parse_address(value: &str) -> Result<Address> {
    Address::from_str(value.trim()).with_context(|| format!("invalid address: {value}"))
}

fn unix_now_secs() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before unix epoch")?
        .as_secs())
}

fn unix_now_millis() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before unix epoch")?
        .as_millis() as u64)
}

fn log_route_error(ctx: ErrorEmitContext<'_>, reason: &str, detail: String, stats: &mut ShadowStats) {
    for input in ctx.input_sizes {
        let row_reason = format!("{reason}:{detail}");
        let row = ShadowDecisionLog {
            run_id: ctx.run_id.to_string(),
            ts_unix_ms: unix_now_millis().unwrap_or(0),
            network: ctx.network.to_string(),
            route: ctx.route.to_string(),
            block: ctx.block,
            block_age_secs: ctx.block_age_secs,
            input_wei: input.to_string(),
            gas_price_wei: "0".to_string(),
            gas_cost_wei: "0".to_string(),
            flash_fee_wei: "0".to_string(),
            total_cost_wei: "0".to_string(),
            v2_out_mid_wei: "0".to_string(),
            v3_out_wei: "0".to_string(),
            net_wei: "0".to_string(),
            edge_bps: "0".to_string(),
            v3_quote_latency_ms: 0,
            decision: "would_skip".to_string(),
            reason: row_reason.clone(),
        };
        if let Ok(json) = serde_json::to_string(&row) {
            println!("{json}");
            stats.record("would_skip", &row_reason);
        }
    }
}

fn emit_row(ctx: EmitContext<'_>, decision: &str, reason: &str, stats: &mut ShadowStats) -> Result<()> {
    let total_cost = ctx
        .input
        .saturating_add(ctx.flash_fee)
        .saturating_add(ctx.gas_cost);
    let net = if ctx.v3_out > total_cost {
        ctx.v3_out - total_cost
    } else {
        U256::zero()
    };
    let edge_bps = signed_edge_bps(ctx.v3_out, total_cost);

    let row = ShadowDecisionLog {
        run_id: ctx.run_id.to_string(),
        ts_unix_ms: unix_now_millis()?,
        network: ctx.network.to_string(),
        route: ctx.route.to_string(),
        block: ctx.block,
        block_age_secs: ctx.block_age_secs,
        input_wei: ctx.input.to_string(),
        gas_price_wei: ctx.gas_price.to_string(),
        gas_cost_wei: ctx.gas_cost.to_string(),
        flash_fee_wei: ctx.flash_fee.to_string(),
        total_cost_wei: total_cost.to_string(),
        v2_out_mid_wei: ctx.v2_out_mid.to_string(),
        v3_out_wei: ctx.v3_out.to_string(),
        net_wei: net.to_string(),
        edge_bps,
        v3_quote_latency_ms: ctx.v3_quote_latency_ms,
        decision: decision.to_string(),
        reason: reason.to_string(),
    };
    println!("{}", serde_json::to_string(&row).context("failed to serialize shadow log row")?);
    stats.record(decision, reason);
    Ok(())
}

fn normalized_reason(reason: &str) -> String {
    reason.split(':').next().unwrap_or(reason).to_string()
}

fn signed_edge_bps(output: U256, total_cost: U256) -> String {
    if total_cost.is_zero() {
        return "0".to_string();
    }

    let precision = U256::from(10_000_u64);
    if output >= total_cost {
        let gain = output - total_cost;
        let bps = gain
            .saturating_mul(precision)
            .checked_div(total_cost)
            .unwrap_or_else(U256::zero);
        bps.to_string()
    } else {
        let loss = total_cost - output;
        let bps = loss
            .saturating_mul(precision)
            .checked_div(total_cost)
            .unwrap_or_else(U256::zero);
        format!("-{bps}")
    }
}

fn top_reason_counts(stats: &ShadowStats, limit: usize) -> Vec<ReasonCount> {
    let mut entries: Vec<(String, u64)> = stats
        .reason_counts
        .iter()
        .map(|(reason, count)| (reason.clone(), *count))
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries
        .into_iter()
        .take(limit)
        .map(|(reason, count)| ReasonCount { reason, count })
        .collect()
}

fn emit_summary(run_id: &str, network: &str, route: &str, latest_block: u64, summary_kind: &str, stats: &ShadowStats) {
    let summary = ShadowSummaryLog {
        run_id: run_id.to_string(),
        network: network.to_string(),
        route: route.to_string(),
        summary_kind: summary_kind.to_string(),
        latest_block,
        blocks_seen: stats.blocks_seen,
        rows_emitted: stats.rows_emitted,
        would_trade: stats.would_trade,
        would_skip: stats.would_skip,
        top_reasons: top_reason_counts(stats, 5),
    };
    match serde_json::to_string(&summary) {
        Ok(json) => eprintln!("{json}"),
        Err(err) => eprintln!("summary serialization failed: {}", sanitize_error(&err)),
    }
}
