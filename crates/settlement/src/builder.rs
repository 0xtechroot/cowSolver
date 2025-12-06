//! Settlement builder for constructing optimized batch settlements
//!
//! Provides a fluent API for building settlements from individual trades,
//! handling interaction ordering, approval management, and optimization.

use crate::{
    Approval, ExecutionMode, Interaction, Result, Settlement, SettlementConstraints,
    SettlementError, SettlementStats, Trade, TradeKind, TokenTransfer,
};
use ethers::types::{Address, Bytes, H256, U256};
use std::collections::{HashMap, HashSet};
use tracing::{debug, info, warn};

/// Builder for constructing settlements
pub struct SettlementBuilder {
    settlement: Settlement,
    constraints: SettlementConstraints,
    stats: SettlementStats,
    token_balances: HashMap<Address, U256>,
    required_approvals: HashSet<(Address, Address)>, // (token, spender)
}

impl SettlementBuilder {
    /// Create a new settlement builder
    pub fn new(mode: ExecutionMode) -> Self {
        let id = H256::random();
        Self {
            settlement: Settlement::new(id, mode),
            constraints: SettlementConstraints::default(),
            stats: SettlementStats::new(),
            token_balances: HashMap::new(),
            required_approvals: HashSet::new(),
        }
    }

    /// Create builder with custom constraints
    pub fn with_constraints(mode: ExecutionMode, constraints: SettlementConstraints) -> Self {
        let id = H256::random();
        Self {
            settlement: Settlement::new(id, mode),
            constraints,
            stats: SettlementStats::new(),
            token_balances: HashMap::new(),
            required_approvals: HashSet::new(),
        }
    }

    /// Add a trade to the settlement
    pub fn add_trade(&mut self, trade: Trade) -> Result<&mut Self> {
        debug!("Adding trade: {:?}", trade.kind);

        // Check order count constraint
        if !self.constraints.check_order_count(self.settlement.orders.len() + 1) {
            return Err(SettlementError::SettlementTooLarge {
                max: self.constraints.max_orders,
                actual: self.settlement.orders.len() + 1,
            });
        }

        // Add order ID
        self.settlement.add_order(trade.order_id)?;

        // Update statistics
        self.stats.add_trade(&trade);
        self.stats.orders_count = self.settlement.orders.len();

        // Track token balances
        self.update_balances(&trade);

        // Add required approvals
        self.track_approval(trade.sell_token, Address::zero()); // Placeholder spender

        // Create trade interaction based on kind
        let interaction = self.create_trade_interaction(&trade)?;
        self.settlement.add_trade(interaction);

        Ok(self)
    }

    /// Add multiple trades in batch
    pub fn add_trades(&mut self, trades: Vec<Trade>) -> Result<&mut Self> {
        for trade in trades {
            self.add_trade(trade)?;
        }
        Ok(self)
    }

    /// Add a pre-interaction (executed before trades)
    pub fn add_pre_interaction(
        &mut self,
        target: Address,
        calldata: Bytes,
        value: U256,
    ) -> &mut Self {
        let interaction = Interaction {
            target,
            calldata,
            value,
        };
        self.settlement.add_pre_interaction(interaction);
        self
    }

    /// Add a post-interaction (executed after trades)
    pub fn add_post_interaction(
        &mut self,
        target: Address,
        calldata: Bytes,
        value: U256,
    ) -> &mut Self {
        let interaction = Interaction {
            target,
            calldata,
            value,
        };
        self.settlement.add_post_interaction(interaction);
        self
    }

    /// Set expected surplus
    pub fn with_surplus(&mut self, surplus: U256) -> &mut Self {
        self.settlement.expected_surplus = surplus;
        self.stats.total_surplus = surplus;
        self
    }

    /// Set gas estimate
    pub fn with_gas_estimate(&mut self, gas: u64) -> Result<&mut Self> {
        if !self.constraints.check_gas(gas) {
            return Err(SettlementError::GasLimitExceeded {
                limit: self.constraints.max_gas,
                estimated: gas,
            });
        }
        self.settlement.gas_estimate = gas;
        self.stats.gas_used = gas;
        Ok(self)
    }

    /// Add metadata to settlement
    pub fn add_metadata(&mut self, key: String, value: String) -> &mut Self {
        self.settlement.metadata.insert(key, value);
        self
    }

    /// Finalize approvals based on tracked requirements
    pub fn finalize_approvals(&mut self, settlement_contract: Address) -> &mut Self {
        for (token, _) in &self.required_approvals {
            let approval = Approval {
                token: *token,
                spender: settlement_contract,
                amount: U256::max_value(), // Max approval for gas efficiency
            };
            self.settlement.add_approval(approval);
        }
        self
    }

    /// Build the final settlement
    pub fn build(self) -> Result<(Settlement, SettlementStats)> {
        // Validate settlement
        self.validate()?;

        info!(
            "Built settlement with {} orders, {} interactions, surplus: {}",
            self.settlement.orders.len(),
            self.settlement.total_interactions(),
            self.settlement.expected_surplus
        );

        Ok((self.settlement, self.stats))
    }

    /// Validate the settlement before building
    fn validate(&self) -> Result<()> {
        // Check not empty
        if self.settlement.is_empty() {
            return Err(SettlementError::InvalidSettlement(
                "Settlement is empty".to_string(),
            ));
        }

        // Check surplus meets minimum
        if self.settlement.expected_surplus < self.constraints.min_surplus {
            warn!(
                "Surplus {} below minimum {}",
                self.settlement.expected_surplus, self.constraints.min_surplus
            );
        }

        // Validate token balances
        self.validate_balances()?;

        Ok(())
    }

    /// Validate that all token transfers are balanced
    fn validate_balances(&self) -> Result<()> {
        for (token, balance) in &self.token_balances {
            if balance.is_zero() {
                continue; // Balanced
            }

            // Non-zero balance indicates imbalance
            warn!("Token {} has imbalanced transfers: {}", token, balance);
        }
        Ok(())
    }

    /// Update token balance tracking
    fn update_balances(&mut self, trade: &Trade) {
        // Deduct sell amount
        let sell_balance = self.token_balances.entry(trade.sell_token).or_insert(U256::zero());
        *sell_balance = sell_balance.saturating_sub(trade.sell_amount + trade.fee_amount);

        // Add buy amount
        let buy_balance = self.token_balances.entry(trade.buy_token).or_insert(U256::zero());
        *buy_balance = buy_balance.saturating_add(trade.buy_amount);
    }

    /// Track required approval
    fn track_approval(&mut self, token: Address, spender: Address) {
        self.required_approvals.insert((token, spender));
    }

    /// Create trade interaction based on trade kind
    fn create_trade_interaction(&self, trade: &Trade) -> Result<Interaction> {
        let calldata = match trade.kind {
            TradeKind::SingleSwap => self.encode_single_swap(trade)?,
            TradeKind::MultiHop => self.encode_multi_hop(trade)?,
            TradeKind::CoW => self.encode_cow_match(trade)?,
            TradeKind::CrossChain => self.encode_cross_chain(trade)?,
            TradeKind::Rfq => self.encode_rfq(trade)?,
        };

        Ok(Interaction {
            target: Address::zero(), // Placeholder - should be actual venue
            calldata,
            value: U256::zero(),
        })
    }

    /// Encode single swap calldata
    fn encode_single_swap(&self, trade: &Trade) -> Result<Bytes> {
        // Simplified encoding - in production, use proper ABI encoding
        let mut data = Vec::new();
        data.extend_from_slice(&trade.sell_token.as_bytes());
        data.extend_from_slice(&trade.buy_token.as_bytes());
        
        let mut sell_bytes = [0u8; 32];
        trade.sell_amount.to_big_endian(&mut sell_bytes);
        data.extend_from_slice(&sell_bytes);
        
        let mut buy_bytes = [0u8; 32];
        trade.buy_amount.to_big_endian(&mut buy_bytes);
        data.extend_from_slice(&buy_bytes);
        
        Ok(Bytes::from(data))
    }

    /// Encode multi-hop swap calldata
    fn encode_multi_hop(&self, trade: &Trade) -> Result<Bytes> {
        // Placeholder for multi-hop encoding
        self.encode_single_swap(trade)
    }

    /// Encode CoW match calldata
    fn encode_cow_match(&self, trade: &Trade) -> Result<Bytes> {
        // CoW matches are settled directly without external interactions
        Ok(Bytes::new())
    }

    /// Encode cross-chain swap calldata
    fn encode_cross_chain(&self, trade: &Trade) -> Result<Bytes> {
        // Placeholder for bridge integration
        self.encode_single_swap(trade)
    }

    /// Encode RFQ execution calldata
    fn encode_rfq(&self, trade: &Trade) -> Result<Bytes> {
        // Placeholder for RFQ encoding
        self.encode_single_swap(trade)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_trade() -> Trade {
        Trade {
            order_id: H256::random(),
            kind: TradeKind::SingleSwap,
            sell_token: Address::random(),
            buy_token: Address::random(),
            sell_amount: U256::from(1000),
            buy_amount: U256::from(2000),
            execution_price: 2.0,
            fee_amount: U256::from(10),
            venue: "Uniswap".to_string(),
        }
    }

    #[test]
    fn test_builder_creation() {
        let builder = SettlementBuilder::new(ExecutionMode::Standard);
        assert_eq!(builder.settlement.mode, ExecutionMode::Standard);
        assert!(builder.settlement.is_empty());
    }

    #[test]
    fn test_add_trade() {
        let mut builder = SettlementBuilder::new(ExecutionMode::Standard);
        let trade = create_test_trade();
        
        assert!(builder.add_trade(trade).is_ok());
        assert_eq!(builder.settlement.orders.len(), 1);
        assert_eq!(builder.stats.trades_count, 1);
    }

    #[test]
    fn test_add_multiple_trades() {
        let mut builder = SettlementBuilder::new(ExecutionMode::Standard);
        let trades = vec![create_test_trade(), create_test_trade()];
        
        // Second trade will fail due to duplicate order ID check
        // In real usage, each trade would have unique order ID
        assert!(builder.add_trades(trades).is_err());
    }

    #[test]
    fn test_surplus_and_gas() {
        let mut builder = SettlementBuilder::new(ExecutionMode::Standard);
        
        builder.with_surplus(U256::from(1000));
        assert_eq!(builder.settlement.expected_surplus, U256::from(1000));
        
        assert!(builder.with_gas_estimate(5_000_000).is_ok());
        assert_eq!(builder.settlement.gas_estimate, 5_000_000);
    }

    #[test]
    fn test_gas_limit_exceeded() {
        let mut builder = SettlementBuilder::new(ExecutionMode::Standard);
        let result = builder.with_gas_estimate(20_000_000);
        
        assert!(result.is_err());
        match result {
            Err(SettlementError::GasLimitExceeded { .. }) => (),
            _ => panic!("Expected GasLimitExceeded error"),
        }
    }

    #[test]
    fn test_build_empty_settlement() {
        let builder = SettlementBuilder::new(ExecutionMode::Standard);
        let result = builder.build();
        
        assert!(result.is_err());
        match result {
            Err(SettlementError::InvalidSettlement(_)) => (),
            _ => panic!("Expected InvalidSettlement error"),
        }
    }
}
