use anyhow::{Context, Result};
use ethers::abi::{ParamType, Token, decode};
use ethers::types::{Address, Bytes, H256, U256};
use ethers::utils::{format_units, keccak256};
use serde::Serialize;

#[derive(Clone, Debug)]
pub enum MarketKind {
    V2Sync,
    V3Swap,
}

#[derive(Clone, Debug)]
pub struct Market {
    pub kind: MarketKind,
    pub pool: Address,
    pub token0: Address,
    pub token1: Address,
    pub token0_symbol: String,
    pub token1_symbol: String,
    pub token0_decimals: u8,
    pub token1_decimals: u8,
}

#[derive(Debug, Serialize)]
pub struct V2NormalizedUpdate {
    pub event: String,
    pub block: u64,
    pub pool: String,
    pub token0: String,
    pub token1: String,
    pub reserve0: String,
    pub reserve1: String,
    pub price_token1_per_token0: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct V3SwapNormalizedUpdate {
    pub event: String,
    pub block: u64,
    pub pool: String,
    pub token0: String,
    pub token1: String,
    pub amount0: String,
    pub amount1: String,
    pub sqrt_price_x96: String,
    pub tick: i32,
    pub price_token1_per_token0: Option<f64>,
}

impl Market {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: MarketKind,
        pool: Address,
        token0: Address,
        token1: Address,
        token0_symbol: String,
        token1_symbol: String,
        token0_decimals: u8,
        token1_decimals: u8,
    ) -> Self {
        Self {
            kind,
            pool,
            token0,
            token1,
            token0_symbol,
            token1_symbol,
            token0_decimals,
            token1_decimals,
        }
    }

    pub fn normalize_v2_sync(&self, block: u64, reserve0: U256, reserve1: U256) -> V2NormalizedUpdate {
        V2NormalizedUpdate {
            event: "v2_sync".to_string(),
            block,
            pool: format!("{:#x}", self.pool),
            token0: self.token0_symbol.clone(),
            token1: self.token1_symbol.clone(),
            reserve0: format_unsigned_amount(reserve0, self.token0_decimals),
            reserve1: format_unsigned_amount(reserve1, self.token1_decimals),
            price_token1_per_token0: v2_price(reserve0, reserve1, self.token0_decimals, self.token1_decimals),
        }
    }

    pub fn normalize_v3_swap(
        &self,
        block: u64,
        amount0_raw: U256,
        amount1_raw: U256,
        sqrt_price_x96: U256,
        tick: i32,
    ) -> V3SwapNormalizedUpdate {
        V3SwapNormalizedUpdate {
            event: "v3_swap".to_string(),
            block,
            pool: format!("{:#x}", self.pool),
            token0: self.token0_symbol.clone(),
            token1: self.token1_symbol.clone(),
            amount0: format_signed_amount(amount0_raw, self.token0_decimals),
            amount1: format_signed_amount(amount1_raw, self.token1_decimals),
            sqrt_price_x96: sqrt_price_x96.to_string(),
            tick,
            price_token1_per_token0: v3_price(sqrt_price_x96, self.token0_decimals, self.token1_decimals),
        }
    }
}

pub fn v2_sync_topic() -> H256 {
    H256::from(keccak256("Sync(uint112,uint112)"))
}

pub fn v3_swap_topic() -> H256 {
    H256::from(keccak256(
        "Swap(address,address,int256,int256,uint160,uint128,int24)",
    ))
}

pub fn decode_v2_sync(data: &Bytes) -> Result<(U256, U256)> {
    let tokens = decode(&[ParamType::Uint(112), ParamType::Uint(112)], data.as_ref())
        .context("Failed to decode V2 Sync event data")?;
    if tokens.len() != 2 {
        anyhow::bail!("Unexpected token length for V2 Sync: {}", tokens.len());
    }

    Ok((token_as_uint(&tokens[0])?, token_as_uint(&tokens[1])?))
}

pub fn decode_v3_swap(data: &Bytes) -> Result<(U256, U256, U256, U256, i32)> {
    let tokens = decode(
        &[
            ParamType::Int(256),
            ParamType::Int(256),
            ParamType::Uint(160),
            ParamType::Uint(128),
            ParamType::Int(24),
        ],
        data.as_ref(),
    )
    .context("Failed to decode V3 Swap event data")?;

    if tokens.len() != 5 {
        anyhow::bail!("Unexpected token length for V3 Swap: {}", tokens.len());
    }

    let amount0_raw = token_as_int_raw(&tokens[0])?;
    let amount1_raw = token_as_int_raw(&tokens[1])?;
    let sqrt_price_x96 = token_as_uint(&tokens[2])?;
    let liquidity = token_as_uint(&tokens[3])?;
    let tick = token_as_int24(&tokens[4])?;

    Ok((amount0_raw, amount1_raw, sqrt_price_x96, liquidity, tick))
}

fn token_as_uint(token: &Token) -> Result<U256> {
    match token {
        Token::Uint(value) => Ok(*value),
        _ => anyhow::bail!("Expected uint token, found {token:?}"),
    }
}

fn token_as_int_raw(token: &Token) -> Result<U256> {
    match token {
        Token::Int(value) => Ok(*value),
        _ => anyhow::bail!("Expected int token, found {token:?}"),
    }
}

fn token_as_int24(token: &Token) -> Result<i32> {
    match token {
        Token::Int(value) => {
            let raw = value.low_u32() & 0x00FF_FFFF;
            let signed = if (raw & (1 << 23)) != 0 {
                (raw as i32) - (1 << 24)
            } else {
                raw as i32
            };
            Ok(signed)
        }
        _ => anyhow::bail!("Expected int24 token, found {token:?}"),
    }
}

fn format_unsigned_amount(raw: U256, decimals: u8) -> String {
    format_units(raw, decimals as usize).unwrap_or_else(|_| raw.to_string())
}

fn format_signed_amount(raw: U256, decimals: u8) -> String {
    if is_negative_int256(raw) {
        let abs = int256_abs(raw);
        let human = format_units(abs, decimals as usize).unwrap_or_else(|_| abs.to_string());
        format!("-{human}")
    } else {
        format_unsigned_amount(raw, decimals)
    }
}

fn is_negative_int256(value: U256) -> bool {
    value.bit(255)
}

fn int256_abs(value: U256) -> U256 {
    if is_negative_int256(value) {
        (!value).overflowing_add(U256::one()).0
    } else {
        value
    }
}

fn v2_price(reserve0: U256, reserve1: U256, decimals0: u8, decimals1: u8) -> Option<f64> {
    let r0 = scale(reserve0, decimals0)?;
    let r1 = scale(reserve1, decimals1)?;
    if r0 == 0.0 {
        None
    } else {
        Some(r1 / r0)
    }
}

fn v3_price(sqrt_price_x96: U256, decimals0: u8, decimals1: u8) -> Option<f64> {
    let sqrt = as_f64(sqrt_price_x96)?;
    let ratio = (sqrt * sqrt) / 2_f64.powi(192);
    let decimal_adjustment = 10_f64.powi(i32::from(decimals0) - i32::from(decimals1));
    Some(ratio * decimal_adjustment)
}

fn scale(raw: U256, decimals: u8) -> Option<f64> {
    as_f64(raw).map(|value| value / 10_f64.powi(i32::from(decimals)))
}

fn as_f64(value: U256) -> Option<f64> {
    value.to_string().parse::<f64>().ok()
}
