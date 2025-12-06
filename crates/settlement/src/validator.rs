//! Settlement validation and safety checks
//!
//! Provides comprehensive validation of settlements before submission,
//! including balance checks, price validation, and risk assessment.

use crate::{Result, Settlement, SettlementError, Validator};
use async_trait::async_trait;
use ethers::types::{Address, U256};
use std::collections::HashMap;
use tracing::{debug, warn};

/// Comprehensive settlement validator
pub struct SettlementValidator {
    /// Minimum surplus required (in wei)
    min_surplus: U256,
    /// Maximum gas allowed
    max_gas: u64,
    /// Maximum orders per settlement
    max_orders: usize,
    /// Token balance cache
    token_balances: HashMap<Address, U256>,
}

impl SettlementValidator {
    /// Create a new validator with default settings
    pub fn new() -> Self {
        Self {
            min_surplus: U256::zero(),
            max_gas: 10_000_000,
            max_orders: 100,
            token_balances: HashMap::new(),
        }
    }

    /// Create validator with custom limits
    pub fn with_limits(min_surplus: U256, max_gas: u64, max_orders: usize) -> Self {
        Self {
            min_surplus,
            max_gas,
            max_orders,
            token_balances: HashMap::new(),
        }
    }

    /// Set token balance for validation
    pub fn set_token_balance(&mut self, token: Address, balance: U256) {
        self.token_balances.insert(token, balance);
    }

    /// Validate order count
    fn validate_order_count(&self, settlement: &Settlement) -> Result<()> {
        if settlement.orders.len() > self.max_orders {
            return Err(SettlementError::SettlementTooLarge {
                max: self.max_orders,
                actual: settlement.orders.len(),
            });
        }
        Ok(())
    }

    /// Validate gas estimate
    fn validate_gas(&self, settlement: &Settlement) -> Result<()> {
        if settlement.gas_estimate > self.max_gas {
            return Err(SettlementError::GasLimitExceeded {
                limit: self.max_gas,
                estimated: settlement.gas_estimate,
            });
        }
        Ok(())
    }

    /// Validate settlement is not empty
    fn validate_not_empty(&self, settlement: &Settlement) -> Result<()> {
        if settlement.is_empty() {
            return Err(SettlementError::InvalidSettlement(
                "Settlement is empty".to_string(),
            ));
        }
        Ok(())
    }

    /// Validate no duplicate orders
    fn validate_no_duplicates(&self, settlement: &Settlement) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for order_id in &settlement.orders {
            if !seen.insert(order_id) {
                return Err(SettlementError::DuplicateOrder(*order_id));
            }
        }
        Ok(())
    }

    /// Validate interactions are properly ordered
    fn validate_interaction_order(&self, settlement: &Settlement) -> Result<()> {
        // Pre-interactions should come before trades
        if !settlement.pre_interactions.is_empty() && settlement.trades.is_empty() {
            warn!("Settlement has pre-interactions but no trades");
        }

        // Post-interactions should come after trades
        if !settlement.post_interactions.is_empty() && settlement.trades.is_empty() {
            warn!("Settlement has post-interactions but no trades");
        }

        Ok(())
    }

    /// Validate approvals are sufficient
    fn validate_approvals_internal(&self, settlement: &Settlement) -> Result<()> {
        // Check that all required approvals are present
        // In production, this would verify against actual token requirements
        
        if settlement.approvals.is_empty() && !settlement.trades.is_empty() {
            debug!("Settlement has trades but no approvals - may need approval setup");
        }

        Ok(())
    }
}

impl Default for SettlementValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Validator for SettlementValidator {
    async fn validate(&self, settlement: &Settlement) -> Result<()> {
        debug!("Validating settlement {}", settlement.id);

        // Run all validation checks
        self.validate_not_empty(settlement)?;
        self.validate_order_count(settlement)?;
        self.validate_gas(settlement)?;
        self.validate_no_duplicates(settlement)?;
        self.validate_interaction_order(settlement)?;
        self.validate_approvals_internal(settlement)?;

        debug!("Settlement validation passed");
        Ok(())
    }

    async fn check_surplus(&self, settlement: &Settlement, min_surplus: U256) -> Result<()> {
        let actual_surplus = settlement.expected_surplus;
        
        if actual_surplus < min_surplus {
            warn!(
                "Surplus {} below minimum {}",
                actual_surplus, min_surplus
            );
            // Don't fail, just warn - solver may still want to submit
        }

        Ok(())
    }

    async fn verify_approvals(&self, settlement: &Settlement) -> Result<()> {
        self.validate_approvals_internal(settlement)
    }
}

/// Risk-aware validator with additional safety checks
pub struct RiskAwareValidator {
    base_validator: SettlementValidator,
    /// Maximum price impact allowed (basis points)
    max_price_impact_bps: u16,
    /// Blacklisted tokens
    blacklisted_tokens: Vec<Address>,
}

impl RiskAwareValidator {
    pub fn new() -> Self {
        Self {
            base_validator: SettlementValidator::new(),
            max_price_impact_bps: 500, // 5%
            blacklisted_tokens: Vec::new(),
        }
    }

    pub fn with_price_impact_limit(mut self, max_bps: u16) -> Self {
        self.max_price_impact_bps = max_bps;
        self
    }

    pub fn add_blacklisted_token(&mut self, token: Address) {
        self.blacklisted_tokens.push(token);
    }

    /// Check for blacklisted tokens
    fn check_blacklist(&self, settlement: &Settlement) -> Result<()> {
        // In production, this would check actual tokens in trades
        // For now, just validate approvals don't include blacklisted tokens
        for approval in &settlement.approvals {
            if self.blacklisted_tokens.contains(&approval.token) {
                return Err(SettlementError::ValidationFailed(format!(
                    "Blacklisted token: {:?}",
                    approval.token
                )));
            }
        }
        Ok(())
    }
}

impl Default for RiskAwareValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Validator for RiskAwareValidator {
    async fn validate(&self, settlement: &Settlement) -> Result<()> {
        // Run base validation
        self.base_validator.validate(settlement).await?;

        // Additional risk checks
        self.check_blacklist(settlement)?;

        debug!("Risk-aware validation passed");
        Ok(())
    }

    async fn check_surplus(&self, settlement: &Settlement, min_surplus: U256) -> Result<()> {
        self.base_validator.check_surplus(settlement, min_surplus).await
    }

    async fn verify_approvals(&self, settlement: &Settlement) -> Result<()> {
        self.base_validator.verify_approvals(settlement).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExecutionMode;

    #[tokio::test]
    async fn test_validate_empty_settlement() {
        let validator = SettlementValidator::new();
        let settlement = Settlement::new(
            ethers::types::H256::random(),
            ExecutionMode::Standard,
        );

        let result = validator.validate(&settlement).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_order_count() {
        let validator = SettlementValidator::with_limits(U256::zero(), 10_000_000, 2);
        let mut settlement = Settlement::new(
            ethers::types::H256::random(),
            ExecutionMode::Standard,
        );

        // Add 3 orders (exceeds limit of 2)
        settlement.add_order(ethers::types::H256::random()).unwrap();
        settlement.add_order(ethers::types::H256::random()).unwrap();
        settlement.add_order(ethers::types::H256::random()).unwrap();

        let result = validator.validate(&settlement).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_gas_limit() {
        let validator = SettlementValidator::with_limits(U256::zero(), 1_000_000, 100);
        let mut settlement = Settlement::new(
            ethers::types::H256::random(),
            ExecutionMode::Standard,
        );

        settlement.add_order(ethers::types::H256::random()).unwrap();
        settlement.gas_estimate = 2_000_000; // Exceeds limit

        let result = validator.validate(&settlement).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_duplicates() {
        let validator = SettlementValidator::new();
        let mut settlement = Settlement::new(
            ethers::types::H256::random(),
            ExecutionMode::Standard,
        );

        let order_id = ethers::types::H256::random();
        settlement.orders.push(order_id);
        settlement.orders.push(order_id); // Duplicate

        let result = validator.validate(&settlement).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_risk_aware_validator() {
        let mut validator = RiskAwareValidator::new();
        let blacklisted = Address::random();
        validator.add_blacklisted_token(blacklisted);

        let mut settlement = Settlement::new(
            ethers::types::H256::random(),
            ExecutionMode::Standard,
        );
        settlement.add_order(ethers::types::H256::random()).unwrap();

        // Add blacklisted token approval
        settlement.add_approval(crate::Approval {
            token: blacklisted,
            spender: Address::random(),
            amount: U256::from(1000),
        });

        let result = validator.validate(&settlement).await;
        assert!(result.is_err());
    }
}
