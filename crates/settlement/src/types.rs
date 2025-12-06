//! Core types for settlement construction
//!
//! Defines the data structures used throughout the settlement building process,
//! including trade types, token transfers, and settlement metadata.

use ethers::types::{Address, Bytes, H256, U256};
use serde::{Deserialize, Serialize};

/// Type of trade execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeKind {
    /// Direct swap on a single AMM
    SingleSwap,
    /// Multi-hop swap across multiple AMMs
    MultiHop,
    /// Coincidence of Wants (direct order matching)
    CoW,
    /// Cross-chain swap via bridge
    CrossChain,
    /// RFQ (Request for Quote) from market maker
    Rfq,
}

/// Token transfer within a settlement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenTransfer {
    /// Token address (zero address for native token)
    pub token: Address,
    /// Sender address
    pub from: Address,
    /// Recipient address
    pub to: Address,
    /// Transfer amount
    pub amount: U256,
}

/// Trade execution details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    /// Order ID this trade settles
    pub order_id: H256,
    /// Type of trade
    pub kind: TradeKind,
    /// Input token
    pub sell_token: Address,
    /// Output token
    pub buy_token: Address,
    /// Amount of sell token
    pub sell_amount: U256,
    /// Amount of buy token
    pub buy_amount: U256,
    /// Execution price (buy_amount / sell_amount)
    pub execution_price: f64,
    /// Fee amount (in sell token)
    pub fee_amount: U256,
    /// Venue/protocol used for execution
    pub venue: String,
}

impl Trade {
    /// Calculate the effective price after fees
    pub fn effective_price(&self) -> f64 {
        let net_sell = self.sell_amount + self.fee_amount;
        if net_sell.is_zero() {
            return 0.0;
        }
        self.buy_amount.as_u128() as f64 / net_sell.as_u128() as f64
    }

    /// Calculate surplus compared to limit price
    pub fn calculate_surplus(&self, limit_price: f64) -> f64 {
        let execution_price = self.effective_price();
        if execution_price > limit_price {
            (execution_price - limit_price) * self.sell_amount.as_u128() as f64
        } else {
            0.0
        }
    }
}

/// Settlement statistics and metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SettlementStats {
    /// Total number of orders settled
    pub orders_count: usize,
    /// Total number of trades executed
    pub trades_count: usize,
    /// Number of CoW matches
    pub cow_matches: usize,
    /// Total surplus generated (in wei)
    pub total_surplus: U256,
    /// Total fees collected (in wei)
    pub total_fees: U256,
    /// Gas used
    pub gas_used: u64,
    /// Execution time (milliseconds)
    pub execution_time_ms: u64,
}

impl SettlementStats {
    /// Create new empty stats
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a trade to statistics
    pub fn add_trade(&mut self, trade: &Trade) {
        self.trades_count += 1;
        if trade.kind == TradeKind::CoW {
            self.cow_matches += 1;
        }
        self.total_fees = self.total_fees.saturating_add(trade.fee_amount);
    }

    /// Calculate average surplus per order
    pub fn avg_surplus_per_order(&self) -> U256 {
        if self.orders_count == 0 {
            return U256::zero();
        }
        self.total_surplus / U256::from(self.orders_count)
    }

    /// Calculate gas efficiency (orders per gas unit)
    pub fn gas_efficiency(&self) -> f64 {
        if self.gas_used == 0 {
            return 0.0;
        }
        self.orders_count as f64 / self.gas_used as f64
    }
}

/// Settlement constraints and limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementConstraints {
    /// Maximum number of orders per settlement
    pub max_orders: usize,
    /// Maximum gas limit
    pub max_gas: u64,
    /// Minimum surplus required (in wei)
    pub min_surplus: U256,
    /// Maximum price slippage (basis points)
    pub max_slippage_bps: u16,
    /// Settlement deadline (unix timestamp)
    pub deadline: u64,
}

impl Default for SettlementConstraints {
    fn default() -> Self {
        Self {
            max_orders: 100,
            max_gas: 10_000_000,
            min_surplus: U256::zero(),
            max_slippage_bps: 50, // 0.5%
            deadline: u64::MAX,
        }
    }
}

impl SettlementConstraints {
    /// Check if slippage is within tolerance
    pub fn check_slippage(&self, expected: U256, actual: U256) -> bool {
        if expected.is_zero() {
            return true;
        }
        
        let diff = if actual > expected {
            actual - expected
        } else {
            expected - actual
        };
        
        let max_diff = expected * U256::from(self.max_slippage_bps) / U256::from(10000);
        diff <= max_diff
    }

    /// Check if gas is within limit
    pub fn check_gas(&self, estimated: u64) -> bool {
        estimated <= self.max_gas
    }

    /// Check if order count is within limit
    pub fn check_order_count(&self, count: usize) -> bool {
        count <= self.max_orders
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trade_effective_price() {
        let trade = Trade {
            order_id: H256::random(),
            kind: TradeKind::SingleSwap,
            sell_token: Address::random(),
            buy_token: Address::random(),
            sell_amount: U256::from(1000),
            buy_amount: U256::from(2000),
            execution_price: 2.0,
            fee_amount: U256::from(10),
            venue: "Uniswap".to_string(),
        };

        let effective = trade.effective_price();
        assert!(effective > 0.0);
        assert!(effective < 2.0); // Should be less due to fees
    }

    #[test]
    fn test_settlement_stats() {
        let mut stats = SettlementStats::new();
        
        let trade = Trade {
            order_id: H256::random(),
            kind: TradeKind::CoW,
            sell_token: Address::random(),
            buy_token: Address::random(),
            sell_amount: U256::from(1000),
            buy_amount: U256::from(2000),
            execution_price: 2.0,
            fee_amount: U256::from(10),
            venue: "CoW".to_string(),
        };

        stats.add_trade(&trade);
        assert_eq!(stats.trades_count, 1);
        assert_eq!(stats.cow_matches, 1);
        assert_eq!(stats.total_fees, U256::from(10));
    }

    #[test]
    fn test_constraints_slippage() {
        let constraints = SettlementConstraints::default();
        
        let expected = U256::from(1000);
        let actual_ok = U256::from(1004); // 0.4% slippage
        let actual_bad = U256::from(1010); // 1% slippage
        
        assert!(constraints.check_slippage(expected, actual_ok));
        assert!(!constraints.check_slippage(expected, actual_bad));
    }

    #[test]
    fn test_constraints_gas() {
        let constraints = SettlementConstraints::default();
        
        assert!(constraints.check_gas(5_000_000));
        assert!(!constraints.check_gas(15_000_000));
    }
}
