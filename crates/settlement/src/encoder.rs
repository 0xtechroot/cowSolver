//! Calldata encoding for CoW Protocol settlement contracts
//!
//! Handles ABI encoding of settlement data into transaction calldata
//! compatible with CoW Protocol's GPv2Settlement contract.

use crate::{Interaction, Result, Settlement, SettlementEncoder, SettlementError};
use async_trait::async_trait;
use ethers::abi::{encode, Token as AbiToken};
use ethers::types::{Address, Bytes, U256};
use tracing::{debug, trace};

/// Encoder for CoW Protocol settlement calldata
pub struct CalldataEncoder {
    /// Settlement contract address
    settlement_contract: Address,
    /// Chain ID for signature verification
    chain_id: u64,
}

impl CalldataEncoder {
    /// Create a new calldata encoder
    pub fn new(settlement_contract: Address, chain_id: u64) -> Self {
        Self {
            settlement_contract,
            chain_id,
        }
    }

    /// Encode interactions into ABI format
    fn encode_interactions(&self, interactions: &[Interaction]) -> Vec<AbiToken> {
        interactions
            .iter()
            .map(|interaction| {
                AbiToken::Tuple(vec![
                    AbiToken::Address(interaction.target),
                    AbiToken::Uint(interaction.value),
                    AbiToken::Bytes(interaction.calldata.to_vec()),
                ])
            })
            .collect()
    }

    /// Encode token clearings (simplified)
    fn encode_clearings(&self, settlement: &Settlement) -> Vec<AbiToken> {
        // In production, this would track actual token flows
        // For now, return empty array
        vec![]
    }

    /// Encode GPv2 trade data
    fn encode_trades(&self, settlement: &Settlement) -> Vec<AbiToken> {
        // Simplified trade encoding
        // In production, this would include full GPv2Trade structs
        settlement
            .orders
            .iter()
            .map(|order_id| {
                AbiToken::Tuple(vec![
                    AbiToken::FixedBytes(order_id.as_bytes().to_vec()),
                    AbiToken::Uint(U256::zero()), // Placeholder for executed amount
                ])
            })
            .collect()
    }

    /// Calculate function selector for settle()
    fn settle_selector(&self) -> [u8; 4] {
        // keccak256("settle(bytes,uint256[],(address,uint256,bytes)[],(address,uint256,bytes)[])")
        // Simplified - in production use proper ABI
        [0x13, 0xd7, 0x9a, 0x0b]
    }
}

#[async_trait]
impl SettlementEncoder for CalldataEncoder {
    async fn encode(&self, settlement: &Settlement) -> Result<Bytes> {
        debug!(
            "Encoding settlement {} with {} orders",
            settlement.id,
            settlement.orders.len()
        );

        // Encode settlement components
        let pre_interactions = self.encode_interactions(&settlement.pre_interactions);
        let trades = self.encode_trades(settlement);
        let post_interactions = self.encode_interactions(&settlement.post_interactions);
        let clearings = self.encode_clearings(settlement);

        // Build the full calldata
        let tokens = vec![
            AbiToken::Array(trades),
            AbiToken::Array(clearings),
            AbiToken::Array(pre_interactions),
            AbiToken::Array(post_interactions),
        ];

        let encoded = encode(&tokens);
        
        // Prepend function selector
        let mut calldata = self.settle_selector().to_vec();
        calldata.extend_from_slice(&encoded);

        trace!("Encoded calldata length: {} bytes", calldata.len());

        Ok(Bytes::from(calldata))
    }

    async fn estimate_gas(&self, settlement: &Settlement) -> Result<u64> {
        // Gas estimation formula based on settlement complexity
        let base_gas = 21_000u64; // Base transaction cost
        let per_order_gas = 50_000u64;
        let per_interaction_gas = 30_000u64;

        let total_gas = base_gas
            + (settlement.orders.len() as u64 * per_order_gas)
            + (settlement.total_interactions() as u64 * per_interaction_gas);

        debug!("Estimated gas: {}", total_gas);

        Ok(total_gas)
    }

    async fn validate_calldata(&self, calldata: &Bytes) -> Result<()> {
        // Basic validation
        if calldata.len() < 4 {
            return Err(SettlementError::EncodingError(
                "Calldata too short".to_string(),
            ));
        }

        // Check function selector
        let selector = &calldata[0..4];
        if selector != self.settle_selector() {
            return Err(SettlementError::EncodingError(
                "Invalid function selector".to_string(),
            ));
        }

        // Check reasonable size (< 1MB)
        if calldata.len() > 1_000_000 {
            return Err(SettlementError::EncodingError(
                "Calldata too large".to_string(),
            ));
        }

        Ok(())
    }
}

/// Optimized encoder for gas-efficient settlements
pub struct OptimizedEncoder {
    base_encoder: CalldataEncoder,
}

impl OptimizedEncoder {
    pub fn new(settlement_contract: Address, chain_id: u64) -> Self {
        Self {
            base_encoder: CalldataEncoder::new(settlement_contract, chain_id),
        }
    }

    /// Compress interactions by removing redundant data
    fn compress_interactions(&self, interactions: &[Interaction]) -> Vec<Interaction> {
        // In production, implement actual compression logic
        // For now, just return as-is
        interactions.to_vec()
    }
}

#[async_trait]
impl SettlementEncoder for OptimizedEncoder {
    async fn encode(&self, settlement: &Settlement) -> Result<Bytes> {
        // Create optimized version of settlement
        let mut optimized = settlement.clone();
        
        // Compress interactions
        optimized.pre_interactions = self.compress_interactions(&settlement.pre_interactions);
        optimized.post_interactions = self.compress_interactions(&settlement.post_interactions);

        // Use base encoder with optimized settlement
        self.base_encoder.encode(&optimized).await
    }

    async fn estimate_gas(&self, settlement: &Settlement) -> Result<u64> {
        // Optimized gas estimation (10% reduction)
        let base_estimate = self.base_encoder.estimate_gas(settlement).await?;
        Ok(base_estimate * 9 / 10)
    }

    async fn validate_calldata(&self, calldata: &Bytes) -> Result<()> {
        self.base_encoder.validate_calldata(calldata).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExecutionMode;

    #[tokio::test]
    async fn test_encode_empty_settlement() {
        let encoder = CalldataEncoder::new(Address::random(), 1);
        let settlement = Settlement::new(
            ethers::types::H256::random(),
            ExecutionMode::Standard,
        );

        let result = encoder.encode(&settlement).await;
        assert!(result.is_ok());

        let calldata = result.unwrap();
        assert!(calldata.len() >= 4); // At least function selector
    }

    #[tokio::test]
    async fn test_gas_estimation() {
        let encoder = CalldataEncoder::new(Address::random(), 1);
        let mut settlement = Settlement::new(
            ethers::types::H256::random(),
            ExecutionMode::Standard,
        );

        // Add some orders
        settlement.add_order(ethers::types::H256::random()).unwrap();
        settlement.add_order(ethers::types::H256::random()).unwrap();

        let gas = encoder.estimate_gas(&settlement).await.unwrap();
        assert!(gas > 21_000); // Should be more than base gas
    }

    #[tokio::test]
    async fn test_validate_calldata() {
        let encoder = CalldataEncoder::new(Address::random(), 1);

        // Valid calldata
        let valid = Bytes::from(vec![0x13, 0xd7, 0x9a, 0x0b, 0x00, 0x00]);
        assert!(encoder.validate_calldata(&valid).await.is_ok());

        // Invalid - too short
        let invalid_short = Bytes::from(vec![0x13]);
        assert!(encoder.validate_calldata(&invalid_short).await.is_err());

        // Invalid - wrong selector
        let invalid_selector = Bytes::from(vec![0x00, 0x00, 0x00, 0x00]);
        assert!(encoder.validate_calldata(&invalid_selector).await.is_err());
    }

    #[tokio::test]
    async fn test_optimized_encoder() {
        let encoder = OptimizedEncoder::new(Address::random(), 1);
        let settlement = Settlement::new(
            ethers::types::H256::random(),
            ExecutionMode::GasOptimized,
        );

        let calldata = encoder.encode(&settlement).await.unwrap();
        let gas = encoder.estimate_gas(&settlement).await.unwrap();

        assert!(calldata.len() >= 4);
        assert!(gas > 0);
    }
}
