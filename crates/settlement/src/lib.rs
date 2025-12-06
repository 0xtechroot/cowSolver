//! Settlement builder for CoW Protocol solver
//!
//! This module provides comprehensive settlement construction and encoding
//! capabilities for CoW Protocol batch auctions. It handles:
//! - Transaction calldata encoding for settlement contracts
//! - Multi-leg settlement construction (swaps, CoWs, bridges)
//! - Gas optimization and batching strategies
//! - Settlement validation and safety checks

use async_trait::async_trait;
use ethers::types::{Address, Bytes, H256, U256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

pub mod builder;
pub mod encoder;
pub mod types;
pub mod validator;

pub use builder::SettlementBuilder;
pub use encoder::CalldataEncoder;
pub use types::*;
pub use validator::SettlementValidator;

/// Errors that can occur during settlement construction
#[derive(Error, Debug)]
pub enum SettlementError {
    #[error("Invalid settlement: {0}")]
    InvalidSettlement(String),

    #[error("Encoding error: {0}")]
    EncodingError(String),

    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    #[error("Insufficient balance: token={token}, required={required}, available={available}")]
    InsufficientBalance {
        token: Address,
        required: U256,
        available: U256,
    },

    #[error("Price tolerance exceeded: expected={expected}, actual={actual}")]
    PriceToleranceExceeded { expected: U256, actual: U256 },

    #[error("Gas limit exceeded: limit={limit}, estimated={estimated}")]
    GasLimitExceeded { limit: u64, estimated: u64 },

    #[error("Settlement too large: max_orders={max}, actual={actual}")]
    SettlementTooLarge { max: usize, actual: usize },

    #[error("Duplicate order: {0}")]
    DuplicateOrder(H256),

    #[error("Missing interaction: {0}")]
    MissingInteraction(String),

    #[error("Contract error: {0}")]
    ContractError(String),
}

pub type Result<T> = std::result::Result<T, SettlementError>;

/// Settlement execution mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// Standard settlement execution
    Standard,
    /// Optimized for gas efficiency
    GasOptimized,
    /// Optimized for MEV protection
    MevProtected,
}

/// Settlement contract interaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interaction {
    /// Target contract address
    pub target: Address,
    /// Calldata for the interaction
    pub calldata: Bytes,
    /// Value to send with the call
    pub value: U256,
}

/// Token approval for settlement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    /// Token address
    pub token: Address,
    /// Spender address
    pub spender: Address,
    /// Approval amount
    pub amount: U256,
}

/// Complete settlement ready for submission
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settlement {
    /// Unique settlement ID
    pub id: H256,
    /// Orders included in this settlement
    pub orders: Vec<H256>,
    /// Pre-interactions (executed before trades)
    pub pre_interactions: Vec<Interaction>,
    /// Trade interactions
    pub trades: Vec<Interaction>,
    /// Post-interactions (executed after trades)
    pub post_interactions: Vec<Interaction>,
    /// Token approvals needed
    pub approvals: Vec<Approval>,
    /// Estimated gas cost
    pub gas_estimate: u64,
    /// Expected surplus (in wei)
    pub expected_surplus: U256,
    /// Execution mode
    pub mode: ExecutionMode,
    /// Metadata
    pub metadata: HashMap<String, String>,
}

impl Settlement {
    /// Create a new settlement
    pub fn new(id: H256, mode: ExecutionMode) -> Self {
        Self {
            id,
            orders: Vec::new(),
            pre_interactions: Vec::new(),
            trades: Vec::new(),
            post_interactions: Vec::new(),
            approvals: Vec::new(),
            gas_estimate: 0,
            expected_surplus: U256::zero(),
            mode,
            metadata: HashMap::new(),
        }
    }

    /// Add an order to the settlement
    pub fn add_order(&mut self, order_id: H256) -> Result<()> {
        if self.orders.contains(&order_id) {
            return Err(SettlementError::DuplicateOrder(order_id));
        }
        self.orders.push(order_id);
        Ok(())
    }

    /// Add a pre-interaction
    pub fn add_pre_interaction(&mut self, interaction: Interaction) {
        self.pre_interactions.push(interaction);
    }

    /// Add a trade interaction
    pub fn add_trade(&mut self, interaction: Interaction) {
        self.trades.push(interaction);
    }

    /// Add a post-interaction
    pub fn add_post_interaction(&mut self, interaction: Interaction) {
        self.post_interactions.push(interaction);
    }

    /// Add a token approval
    pub fn add_approval(&mut self, approval: Approval) {
        self.approvals.push(approval);
    }

    /// Get total number of interactions
    pub fn total_interactions(&self) -> usize {
        self.pre_interactions.len() + self.trades.len() + self.post_interactions.len()
    }

    /// Check if settlement is empty
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty() && self.total_interactions() == 0
    }
}

/// Trait for settlement encoding strategies
#[async_trait]
pub trait SettlementEncoder: Send + Sync {
    /// Encode settlement into transaction calldata
    async fn encode(&self, settlement: &Settlement) -> Result<Bytes>;

    /// Estimate gas for the settlement
    async fn estimate_gas(&self, settlement: &Settlement) -> Result<u64>;

    /// Validate encoded calldata
    async fn validate_calldata(&self, calldata: &Bytes) -> Result<()>;
}

/// Trait for settlement validation
#[async_trait]
pub trait Validator: Send + Sync {
    /// Validate a settlement before encoding
    async fn validate(&self, settlement: &Settlement) -> Result<()>;

    /// Check if settlement meets minimum surplus requirements
    async fn check_surplus(&self, settlement: &Settlement, min_surplus: U256) -> Result<()>;

    /// Verify all required approvals are present
    async fn verify_approvals(&self, settlement: &Settlement) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settlement_creation() {
        let id = H256::random();
        let settlement = Settlement::new(id, ExecutionMode::Standard);
        
        assert_eq!(settlement.id, id);
        assert_eq!(settlement.mode, ExecutionMode::Standard);
        assert!(settlement.is_empty());
    }

    #[test]
    fn test_add_order() {
        let mut settlement = Settlement::new(H256::random(), ExecutionMode::Standard);
        let order_id = H256::random();
        
        assert!(settlement.add_order(order_id).is_ok());
        assert_eq!(settlement.orders.len(), 1);
        
        // Duplicate should fail
        assert!(settlement.add_order(order_id).is_err());
    }

    #[test]
    fn test_interactions() {
        let mut settlement = Settlement::new(H256::random(), ExecutionMode::Standard);
        
        let interaction = Interaction {
            target: Address::random(),
            calldata: Bytes::from(vec![1, 2, 3]),
            value: U256::zero(),
        };
        
        settlement.add_pre_interaction(interaction.clone());
        settlement.add_trade(interaction.clone());
        settlement.add_post_interaction(interaction);
        
        assert_eq!(settlement.total_interactions(), 3);
        assert!(!settlement.is_empty());
    }
}
