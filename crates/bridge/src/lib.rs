pub mod providers;

use async_trait::async_trait;
use ethers::types::{Address, U256, Bytes};
use serde::{Deserialize, Serialize};
use solver_core::domain::ChainId;

/// Bridge quote for cross-chain transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeQuote {
    /// Bridge provider name
    pub provider: String,
    
    /// Source chain
    pub source_chain: ChainId,
    
    /// Destination chain
    pub destination_chain: ChainId,
    
    /// Token being bridged
    pub token: Address,
    
    /// Amount to bridge
    pub amount: U256,
    
    /// Expected amount on destination (after fees)
    pub expected_output: U256,
    
    /// Bridge fee
    pub fee: U256,
    
    /// Estimated time in seconds
    pub estimated_time: u64,
    
    /// Call data for bridge contract
    pub call_data: Bytes,
}

/// Bridge provider trait
#[async_trait]
pub trait BridgeProvider: Send + Sync {
    /// Returns provider name
    fn name(&self) -> &str;
    
    /// Gets a quote for bridging
    async fn get_quote(
        &self,
        source_chain: ChainId,
        destination_chain: ChainId,
        token: Address,
        amount: U256,
        recipient: Address,
    ) -> anyhow::Result<BridgeQuote>;
    
    /// Checks if route is supported
    async fn supports_route(
        &self,
        source_chain: ChainId,
        destination_chain: ChainId,
        token: Address,
    ) -> bool;
    
    /// Gets bridge contract address for chain
    fn bridge_contract(&self, chain: ChainId) -> Option<Address>;
}

/// Bridge error types
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("Route not supported: {0}")]
    RouteNotSupported(String),
    
    #[error("Insufficient liquidity: {0}")]
    InsufficientLiquidity(String),
    
    #[error("Bridge provider error: {0}")]
    ProviderError(String),
    
    #[error("Quote expired")]
    QuoteExpired,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bridge_quote_creation() {
        let quote = BridgeQuote {
            provider: "Across".to_string(),
            source_chain: ChainId::Ethereum,
            destination_chain: ChainId::Arbitrum,
            token: Address::zero(),
            amount: U256::from(1000),
            expected_output: U256::from(995),
            fee: U256::from(5),
            estimated_time: 300,
            call_data: Bytes::default(),
        };
        
        assert_eq!(quote.provider, "Across");
        assert_eq!(quote.estimated_time, 300);
    }
}
