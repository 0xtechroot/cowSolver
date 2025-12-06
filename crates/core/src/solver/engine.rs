use super::{AuctionContext, Solution, Solver, SolverConfig};
use crate::domain::{Order, OrderId, OrderSide, TokenAmount};
use crate::settlement::SettlementPlan;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Multi-objective optimization objectives for solver
#[derive(Debug, Clone, Copy)]
pub struct Objectives {
    /// User surplus (maximize)
    pub surplus: f64,
    /// Gas cost (minimize)
    pub gas_cost: f64,
    /// Slippage (minimize)
    pub slippage: f64,
    /// Execution risk (minimize)
    pub risk: f64,
}

impl Objectives {
    /// Calculate Pareto dominance score
    /// Higher is better - combines all objectives with appropriate weights
    pub fn pareto_score(&self) -> f64 {
        // Weights based on CoW Protocol's hybrid surplus-sharing model
        const SURPLUS_WEIGHT: f64 = 0.50; // 50% to users
        const GAS_WEIGHT: f64 = 0.25;
        const SLIPPAGE_WEIGHT: f64 = 0.15;
        const RISK_WEIGHT: f64 = 0.10;

        self.surplus * SURPLUS_WEIGHT
            - self.gas_cost * GAS_WEIGHT
            - self.slippage * SLIPPAGE_WEIGHT
            - self.risk * RISK_WEIGHT
    }

    /// Check if this solution Pareto-dominates another
    pub fn dominates(&self, other: &Objectives) -> bool {
        self.surplus >= other.surplus
            && self.gas_cost <= other.gas_cost
            && self.slippage <= other.slippage
            && self.risk <= other.risk
            && (self.surplus > other.surplus
                || self.gas_cost < other.gas_cost
                || self.slippage < other.slippage
                || self.risk < other.risk)
    }
}

/// Execution mode for solver engine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Baseline: solve individual orders via on-chain liquidity
    Baseline,
    /// Naive: batch similar orders and settle net amounts
    Naive,
    /// Optimized: full combinatorial optimization with CoW matching
    Optimized,
}

/// Core solver engine implementing batch auction logic
pub struct SolverEngine {
    config: SolverConfig,
    mode: ExecutionMode,
    /// Cached liquidity state for faster computation
    liquidity_cache: Arc<tokio::sync::RwLock<HashMap<String, f64>>>,
}

impl SolverEngine {
    /// Create new solver engine with configuration
    pub fn new(config: SolverConfig, mode: ExecutionMode) -> Self {
        Self {
            config,
            mode,
            liquidity_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Detect CoW (Coincidence of Wants) opportunities in order batch
    /// Returns pairs of orders that can be matched directly
    fn detect_cow_matches(&self, orders: &[Order]) -> Vec<(OrderId, OrderId)> {
        let mut matches = Vec::new();

        if !self.config.enable_cow_matching {
            return matches;
        }

        // Group orders by token pair
        let mut buy_orders: HashMap<(String, String), Vec<&Order>> = HashMap::new();
        let mut sell_orders: HashMap<(String, String), Vec<&Order>> = HashMap::new();

        for order in orders {
            let pair = (order.sell_token.clone(), order.buy_token.clone());
            match order.side {
                OrderSide::Buy => buy_orders.entry(pair).or_default().push(order),
                OrderSide::Sell => sell_orders.entry(pair).or_default().push(order),
            }
        }

        // Find matching pairs
        for ((sell_token, buy_token), sellers) in &sell_orders {
            let reverse_pair = (buy_token.clone(), sell_token.clone());
            if let Some(buyers) = buy_orders.get(&reverse_pair) {
                for seller in sellers {
                    for buyer in buyers {
                        if self.can_match(seller, buyer) {
                            matches.push((seller.id.clone(), buyer.id.clone()));
                        }
                    }
                }
            }
        }

        matches
    }

    /// Check if two orders can be matched
    fn can_match(&self, order_a: &Order, order_b: &Order) -> bool {
        // Basic price compatibility check
        let price_a = order_a.buy_amount.0 as f64 / order_a.sell_amount.0 as f64;
        let price_b = order_b.buy_amount.0 as f64 / order_b.sell_amount.0 as f64;

        // Prices must overlap for a match
        price_a <= price_b * 1.01 // Allow 1% tolerance
    }

    /// Calculate uniform clearing price for matched orders
    fn calculate_clearing_price(&self, orders: &[Order]) -> f64 {
        if orders.is_empty() {
            return 0.0;
        }

        // Use volume-weighted average price
        let mut total_volume = 0.0;
        let mut weighted_price = 0.0;

        for order in orders {
            let volume = order.sell_amount.0 as f64;
            let price = order.buy_amount.0 as f64 / order.sell_amount.0 as f64;
            weighted_price += price * volume;
            total_volume += volume;
        }

        if total_volume > 0.0 {
            weighted_price / total_volume
        } else {
            0.0
        }
    }

    /// Solve in baseline mode (individual order routing)
    async fn solve_baseline(&self, orders: Vec<Order>) -> crate::Result<Option<Solution>> {
        // Route each order individually through AMMs
        let mut total_gas = 0u64;
        let mut total_surplus = 0.0;
        let order_ids: Vec<OrderId> = orders.iter().map(|o| o.id.clone()).collect();

        for order in &orders {
            // Estimate gas for individual routing
            total_gas += 150_000; // Approximate gas per swap
            
            // Calculate surplus (simplified)
            let surplus = order.buy_amount.0 as f64 * 0.001; // 0.1% surplus
            total_surplus += surplus;
        }

        Ok(Some(Solution {
            orders: order_ids,
            settlement: SettlementPlan::default(),
            gas_cost: total_gas,
            surplus: total_surplus,
            score: total_surplus - (total_gas as f64 * 1e-9),
        }))
    }

    /// Solve in naive mode (batch netting)
    async fn solve_naive(&self, orders: Vec<Order>) -> crate::Result<Option<Solution>> {
        // Group orders by token pair and net them
        let mut net_positions: HashMap<(String, String), i64> = HashMap::new();
        let order_ids: Vec<OrderId> = orders.iter().map(|o| o.id.clone()).collect();

        for order in &orders {
            let pair = (order.sell_token.clone(), order.buy_token.clone());
            let amount = order.sell_amount.0 as i64;
            *net_positions.entry(pair).or_insert(0) += amount;
        }

        // Calculate gas savings from netting
        let num_net_trades = net_positions.len();
        let gas_cost = (num_net_trades as u64) * 100_000; // Reduced gas from batching
        
        // Calculate surplus from netting
        let surplus = orders.len() as f64 * 0.002; // 0.2% surplus from batching

        Ok(Some(Solution {
            orders: order_ids,
            settlement: SettlementPlan::default(),
            gas_cost,
            surplus,
            score: surplus - (gas_cost as f64 * 1e-9),
        }))
    }

    /// Solve in optimized mode (full CoW matching + routing)
    async fn solve_optimized(&self, orders: Vec<Order>) -> crate::Result<Option<Solution>> {
        // Find CoW matches
        let matches = self.detect_cow_matches(&orders);
        
        let mut matched_orders = HashSet::new();
        let mut total_surplus = 0.0;
        
        // Process matches
        for (order_a, order_b) in &matches {
            matched_orders.insert(order_a.clone());
            matched_orders.insert(order_b.clone());
            
            // CoW matches generate significant surplus
            total_surplus += 0.01; // 1% surplus per match
        }

        // Calculate clearing price for matched orders
        let matched_order_refs: Vec<&Order> = orders
            .iter()
            .filter(|o| matched_orders.contains(&o.id))
            .collect();
        
        let _clearing_price = self.calculate_clearing_price(&matched_order_refs);

        // Route remaining orders through AMMs
        let unmatched_count = orders.len() - matched_orders.len();
        let routing_gas = (unmatched_count as u64) * 150_000;
        let matching_gas = (matches.len() as u64) * 50_000; // CoW matches are cheaper
        
        let total_gas = routing_gas + matching_gas;
        let order_ids: Vec<OrderId> = orders.iter().map(|o| o.id.clone()).collect();

        Ok(Some(Solution {
            orders: order_ids,
            settlement: SettlementPlan::default(),
            gas_cost: total_gas,
            surplus: total_surplus,
            score: total_surplus - (total_gas as f64 * 1e-9),
        }))
    }
}

#[async_trait]
impl Solver for SolverEngine {
    async fn solve(&self, orders: Vec<Order>) -> crate::Result<Option<Solution>> {
        if orders.is_empty() {
            return Ok(None);
        }

        // Apply timeout
        let timeout_duration = Duration::from_millis(self.config.timeout_ms);
        
        let result = timeout(timeout_duration, async {
            match self.mode {
                ExecutionMode::Baseline => self.solve_baseline(orders).await,
                ExecutionMode::Naive => self.solve_naive(orders).await,
                ExecutionMode::Optimized => self.solve_optimized(orders).await,
            }
        })
        .await;

        match result {
            Ok(solution) => solution,
            Err(_) => {
                // Timeout occurred
                Ok(None)
            }
        }
    }

    fn name(&self) -> &str {
        match self.mode {
            ExecutionMode::Baseline => "SolverEngine::Baseline",
            ExecutionMode::Naive => "SolverEngine::Naive",
            ExecutionMode::Optimized => "SolverEngine::Optimized",
        }
    }

    fn config(&self) -> &SolverConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{OrderStatus, OrderType};

    fn create_test_order(
        id: u8,
        sell_token: &str,
        buy_token: &str,
        sell_amount: u128,
        buy_amount: u128,
    ) -> Order {
        Order {
            id: OrderId([id; 32]),
            owner: format!("0x{:040x}", id),
            sell_token: sell_token.to_string(),
            buy_token: buy_token.to_string(),
            sell_amount: TokenAmount(sell_amount),
            buy_amount: TokenAmount(buy_amount),
            valid_to: 9999999999,
            fee_amount: TokenAmount(10),
            kind: OrderType::Sell,
            partially_fillable: false,
            status: OrderStatus::Open,
            side: OrderSide::Sell,
            source_chain: None,
            destination_chain: None,
            bridge_provider: None,
        }
    }

    #[tokio::test]
    async fn test_baseline_mode() {
        let config = SolverConfig::default();
        let engine = SolverEngine::new(config, ExecutionMode::Baseline);

        let orders = vec![
            create_test_order(1, "USDC", "WETH", 1000_000000, 500_000000000000000000),
        ];

        let solution = engine.solve(orders).await.unwrap();
        assert!(solution.is_some());
        
        let sol = solution.unwrap();
        assert_eq!(sol.orders.len(), 1);
        assert!(sol.gas_cost > 0);
    }

    #[tokio::test]
    async fn test_naive_mode() {
        let config = SolverConfig::default();
        let engine = SolverEngine::new(config, ExecutionMode::Naive);

        let orders = vec![
            create_test_order(1, "USDC", "WETH", 1000_000000, 500_000000000000000000),
            create_test_order(2, "USDC", "WETH", 2000_000000, 1000_000000000000000000),
        ];

        let solution = engine.solve(orders).await.unwrap();
        assert!(solution.is_some());
        
        let sol = solution.unwrap();
        assert_eq!(sol.orders.len(), 2);
    }

    #[tokio::test]
    async fn test_optimized_mode_with_cow() {
        let config = SolverConfig::default();
        let engine = SolverEngine::new(config, ExecutionMode::Optimized);

        let orders = vec![
            create_test_order(1, "USDC", "WETH", 1000_000000, 500_000000000000000000),
            create_test_order(2, "WETH", "USDC", 500_000000000000000000, 1000_000000),
        ];

        let solution = engine.solve(orders).await.unwrap();
        assert!(solution.is_some());
        
        let sol = solution.unwrap();
        assert_eq!(sol.orders.len(), 2);
        assert!(sol.surplus > 0.0); // Should have CoW surplus
    }

    #[test]
    fn test_objectives_pareto_score() {
        let obj = Objectives {
            surplus: 100.0,
            gas_cost: 10.0,
            slippage: 1.0,
            risk: 0.5,
        };

        let score = obj.pareto_score();
        assert!(score > 0.0);
    }

    #[test]
    fn test_objectives_dominance() {
        let obj1 = Objectives {
            surplus: 100.0,
            gas_cost: 10.0,
            slippage: 1.0,
            risk: 0.5,
        };

        let obj2 = Objectives {
            surplus: 90.0,
            gas_cost: 15.0,
            slippage: 2.0,
            risk: 1.0,
        };

        assert!(obj1.dominates(&obj2));
        assert!(!obj2.dominates(&obj1));
    }
}
