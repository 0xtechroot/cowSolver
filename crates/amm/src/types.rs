//! Core types for AMM routing

use ethers::types::{Address, U256};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Supported AMM protocol types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PoolType {
    UniswapV2,
    UniswapV3,
    Balancer,
    BalancerWeighted,
    BalancerStable,
    Curve,
    CurveStable,
    CurveCrypto,
}

impl fmt::Display for PoolType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PoolType::UniswapV2 => write!(f, "UniswapV2"),
            PoolType::UniswapV3 => write!(f, "UniswapV3"),
            PoolType::Balancer => write!(f, "Balancer"),
            PoolType::BalancerWeighted => write!(f, "BalancerWeighted"),
            PoolType::BalancerStable => write!(f, "BalancerStable"),
            PoolType::Curve => write!(f, "Curve"),
            PoolType::CurveStable => write!(f, "CurveStable"),
            PoolType::CurveCrypto => write!(f, "CurveCrypto"),
        }
    }
}

/// Pool reserves for different AMM types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PoolReserves {
    /// UniswapV2-style constant product (x * y = k)
    ConstantProduct {
        reserve0: U256,
        reserve1: U256,
        token0: Address,
        token1: Address,
    },
    
    /// UniswapV3 concentrated liquidity
    ConcentratedLiquidity {
        liquidity: u128,
        sqrt_price: U256,
        tick: i32,
        token0: Address,
        token1: Address,
        fee: u32,
    },
    
    /// Balancer weighted pool
    Weighted {
        balances: Vec<U256>,
        weights: Vec<U256>,
        tokens: Vec<Address>,
        swap_fee: U256,
    },
    
    /// Balancer stable pool
    Stable {
        balances: Vec<U256>,
        tokens: Vec<Address>,
        amplification: U256,
        swap_fee: U256,
    },
    
    /// Curve stable pool
    CurveStable {
        balances: Vec<U256>,
        tokens: Vec<Address>,
        a: U256,
        fee: U256,
    },
}

/// Represents a swap route through AMM pools
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    /// Ordered list of pool addresses to route through
    pub pools: Vec<Address>,
    
    /// Ordered list of tokens in the route (length = pools.len() + 1)
    pub tokens: Vec<Address>,
    
    /// Pool types for each hop
    pub pool_types: Vec<PoolType>,
    
    /// Expected output amount
    pub amount_out: U256,
    
    /// Total gas cost estimate
    pub gas_cost: u64,
    
    /// Price impact percentage (basis points)
    pub price_impact_bps: u64,
}

impl Route {
    /// Calculate gas-adjusted output (output - gas_cost_in_output_token)
    pub fn gas_adjusted_output(&self, gas_price: U256, output_token_price_usd: U256) -> U256 {
        let gas_cost_wei = U256::from(self.gas_cost) * gas_price;
        let gas_cost_in_output = gas_cost_wei * output_token_price_usd / U256::from(10u128.pow(18));
        
        if self.amount_out > gas_cost_in_output {
            self.amount_out - gas_cost_in_output
        } else {
            U256::zero()
        }
    }
    
    /// Number of hops in the route
    pub fn hop_count(&self) -> usize {
        self.pools.len()
    }
    
    /// Check if route is direct (single hop)
    pub fn is_direct(&self) -> bool {
        self.pools.len() == 1
    }
}

/// Quote for a potential swap
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapQuote {
    /// Input token
    pub token_in: Address,
    
    /// Output token
    pub token_out: Address,
    
    /// Input amount
    pub amount_in: U256,
    
    /// Expected output amount
    pub amount_out: U256,
    
    /// Best route found
    pub route: Route,
    
    /// Minimum output after slippage tolerance
    pub min_amount_out: U256,
    
    /// Slippage tolerance in basis points
    pub slippage_bps: u64,
}

/// Configuration for route finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    /// Maximum number of hops allowed
    pub max_hops: usize,
    
    /// Maximum number of routes to consider
    pub max_routes: usize,
    
    /// Slippage tolerance in basis points (e.g., 50 = 0.5%)
    pub slippage_bps: u64,
    
    /// Gas price for cost calculations
    pub gas_price: U256,
    
    /// Whether to include multi-hop routes
    pub allow_multi_hop: bool,
    
    /// Minimum liquidity threshold
    pub min_liquidity: U256,
}

impl Default for RouteConfig {
    fn default() -> Self {
        Self {
            max_hops: 3,
            max_routes: 5,
            slippage_bps: 50, // 0.5%
            gas_price: U256::from(30_000_000_000u64), // 30 gwei
            allow_multi_hop: true,
            min_liquidity: U256::from(1000u64) * U256::from(10u128.pow(18)), // 1000 tokens
        }
    }
}

/// Pool metadata for routing decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolMetadata {
    pub address: Address,
    pub pool_type: PoolType,
    pub tokens: Vec<Address>,
    pub tvl: U256,
    pub volume_24h: U256,
    pub fee_bps: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_type_display() {
        assert_eq!(PoolType::UniswapV2.to_string(), "UniswapV2");
        assert_eq!(PoolType::Balancer.to_string(), "Balancer");
    }

    #[test]
    fn test_route_hop_count() {
        let route = Route {
            pools: vec![Address::zero(), Address::zero()],
            tokens: vec![Address::zero(), Address::zero(), Address::zero()],
            pool_types: vec![PoolType::UniswapV2, PoolType::UniswapV3],
            amount_out: U256::from(1000),
            gas_cost: 150000,
            price_impact_bps: 30,
        };
        
        assert_eq!(route.hop_count(), 2);
        assert!(!route.is_direct());
    }

    #[test]
    fn test_default_route_config() {
        let config = RouteConfig::default();
        assert_eq!(config.max_hops, 3);
        assert_eq!(config.slippage_bps, 50);
        assert!(config.allow_multi_hop);
    }
}
