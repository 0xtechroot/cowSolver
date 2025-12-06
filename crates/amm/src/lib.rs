//! AMM Routing Module
//!
//! Provides unified interface for routing trades across multiple AMM protocols
//! including Uniswap V2/V3, Balancer, and Curve. Implements graph-based pathfinding
//! with gas-adjusted pricing for optimal route discovery.

pub mod types;
pub mod router;
pub mod pools;
pub mod uniswap;
pub mod balancer;
pub mod curve;

use async_trait::async_trait;
use ethers::types::{Address, U256};
use std::sync::Arc;
use thiserror::Error;

pub use types::*;
pub use router::Router;

/// Errors that can occur during AMM routing operations
#[derive(Error, Debug)]
pub enum AmmError {
    #[error("Insufficient liquidity for swap: {0}")]
    InsufficientLiquidity(String),
    
    #[error("No route found between {from} and {to}")]
    NoRouteFound { from: Address, to: Address },
    
    #[error("Slippage tolerance exceeded: expected {expected}, got {actual}")]
    SlippageExceeded { expected: U256, actual: U256 },
    
    #[error("Pool not found: {0}")]
    PoolNotFound(Address),
    
    #[error("Invalid pool state: {0}")]
    InvalidPoolState(String),
    
    #[error("Gas estimation failed: {0}")]
    GasEstimationFailed(String),
    
    #[error("RPC error: {0}")]
    RpcError(String),
    
    #[error("Calculation overflow")]
    Overflow,
    
    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),
}

pub type Result<T> = std::result::Result<T, AmmError>;

/// Trait for AMM pool implementations
#[async_trait]
pub trait AmmPool: Send + Sync {
    /// Get the pool address
    fn address(&self) -> Address;
    
    /// Get the pool type (UniswapV2, UniswapV3, Balancer, Curve)
    fn pool_type(&self) -> PoolType;
    
    /// Get tokens in the pool
    fn tokens(&self) -> Vec<Address>;
    
    /// Calculate output amount for a given input
    /// Returns (output_amount, gas_cost)
    async fn get_amount_out(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
    ) -> Result<(U256, u64)>;
    
    /// Calculate input amount needed for desired output
    /// Returns (input_amount, gas_cost)
    async fn get_amount_in(
        &self,
        token_in: Address,
        token_out: Address,
        amount_out: U256,
    ) -> Result<(U256, u64)>;
    
    /// Get current reserves/state
    async fn get_reserves(&self) -> Result<PoolReserves>;
    
    /// Update pool state from chain
    async fn sync(&mut self) -> Result<()>;
    
    /// Estimate gas cost for this swap
    fn estimate_gas(&self) -> u64;
}

/// Trait for AMM protocol adapters
#[async_trait]
pub trait AmmAdapter: Send + Sync {
    /// Get all pools for this protocol
    async fn get_pools(&self) -> Result<Vec<Arc<dyn AmmPool>>>;
    
    /// Get pools containing specific tokens
    async fn get_pools_for_tokens(
        &self,
        tokens: &[Address],
    ) -> Result<Vec<Arc<dyn AmmPool>>>;
    
    /// Get a specific pool by address
    async fn get_pool(&self, address: Address) -> Result<Arc<dyn AmmPool>>;
    
    /// Protocol name
    fn name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = AmmError::InsufficientLiquidity("test pool".to_string());
        assert!(err.to_string().contains("Insufficient liquidity"));
    }
}
