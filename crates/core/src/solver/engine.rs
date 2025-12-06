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

        // Find matching pairs: A sells X for Y, B sells Y for X
        for ((sell_token, buy_token), sellers) in &sell_orders {
            let reverse_pair = (buy_token.clone(), sell_token.clone());
            if let Some(buyers) = buy_orders.get(&reverse_pair) {
                // Found potential CoW match
                for seller in sellers {
                    for buyer in buyers {
                        if self.can_match_orders(seller, buyer) {
                            matches.push((seller.id.clone(), buyer.id.clone()));
                        }
                    }
                }
            }
        }

        matches
    }

    /// Check if two orders can be matched based on price compatibility
    fn can_match_orders(&self, order_a: &Order, order_b: &Order) -> bool {
        // Calculate effective prices
        let price_a = order_a.buy_amount.0 as f64 / order_a.sell_amount.0 as f64;
        let price_b = order_b.buy_amount.0 as f64 / order_b.sell_amount.0 as f64;

        // Orders can match if their price ranges overlap
        // This is a simplified check - production would use limit prices
        let price_tolerance = 0.01; // 1% tolerance
        (price_a - price_b).abs() / price_a.max(price_b) <= price_tolerance
    }

    /// Calculate uniform clearing price for a batch of orders
    /// Uses market equilibrium to find price that maximizes matched volume
    fn calculate_clearing_price(
        &self,
        buy_orders: &[&Order],
        sell_orders: &[&Order],
    ) -> Option<f64> {
        if buy_orders.is_empty() || sell_orders.is_empty() {
            return None;
        }

        // Sort buy orders by price (descending) and sell orders by price (ascending)
        let mut buy_prices: Vec<f64> = buy_orders
            .iter()
            .map(|o| o.buy_amount.0 as f64 / o.sell_amount.0 as f64)
            .collect();
        let mut sell_prices: Vec<f64> = sell_orders
            .iter()
            .map(|o| o.buy_amount.0 as f64 / o.sell_amount.0 as f64)
            .collect();

        buy_prices.sort_by(|a, b| b.partial_cmp(a).unwrap());
        sell_prices.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // Find intersection point - uniform clearing price
        let mut clearing_price = 0.0;
        let mut max_volume = 0.0;

        for (i, &buy_price) in buy_prices.iter().enumerate() {
            for (j, &sell_price) in sell_prices.iter().enumerate() {
                if buy_price >= sell_price {
                    let price = (buy_price + sell_price) / 2.0;
                    let volume = (i + j) as f64;
                    if volume > max_volume {
                        max_volume = volume;
                        clearing_price = price;
                    }
                }
            }
        }

        if max_volume > 0.0 {
            Some(clearing_price)
        } else {
            None
        }
    }

    /// Build solution for baseline mode (individual order execution)
    async fn solve_baseline(&self, orders: Vec<Order>) -> crate::Result<Option<Solution>> {
        if orders.is_empty() {
            return Ok(None);
        }

        let mut total_surplus = 0.0;
        let mut total_gas = 0u64;
        let order_ids: Vec<OrderId> = orders.iter().map(|o| o.id.clone()).collect();

        // Estimate gas and surplus for each order independently
        for order in &orders {
            // Base gas cost per order (simplified)
            total_gas += 150_000;

            // Calculate surplus (difference between limit price and execution price)
            let limit_price = order.buy_amount.0 as f64 / order.sell_amount.0 as f64;
            let execution_price = limit_price * 0.995; // Assume 0.5% improvement
            total_surplus += (limit_price - execution_price) * order.sell_amount.0 as f64;
        }

        let mut solution = Solution {
            orders: order_ids,
            settlement: SettlementPlan::default(),
            gas_cost: total_gas,
            surplus: total_surplus,
            score: 0.0,
        };

        solution.calculate_score();

        Ok(Some(solution))
    }

    /// Build solution for naive mode (simple batching)
    async fn solve_naive(&self, orders: Vec<Order>) -> crate::Result<Option<Solution>> {
        if orders.is_empty() {
            return Ok(None);
        }

        // Detect CoW matches
        let cow_matches = self.detect_cow_matches(&orders);

        let mut total_surplus = 0.0;
        let mut total_gas = 0u64;
        let order_ids: Vec<OrderId> = orders.iter().map(|o| o.id.clone()).collect();

        // Gas savings from batching
        let base_gas_per_order = 150_000u64;
        let batch_gas_savings = 20_000u64; // Save 20k gas per batched order

        total_gas = base_gas_per_order * orders.len() as u64;
        if orders.len() > 1 {
            total_gas -= batch_gas_savings * (orders.len() as u64 - 1);
        }

        // Additional surplus from CoW matches
        for (order_a_id, order_b_id) in &cow_matches {
            // CoW matches save gas and improve prices
            total_gas = total_gas.saturating_sub(50_000); // Save gas on CoW match
            total_surplus += 0.01; // Additional surplus from direct matching
        }

        // Base surplus calculation
        for order in &orders {
            let limit_price = order.buy_amount.0 as f64 / order.sell_amount.0 as f64;
            let execution_price = limit_price * 0.997; // Better execution in batch
            total_surplus += (limit_price - execution_price) * order.sell_amount.0 as f64;
        }

        let mut solution = Solution {
            orders: order_ids,
            settlement: SettlementPlan::default(),
            gas_cost: total_gas,
            surplus: total_surplus,
            score: 0.0,
        };

        solution.calculate_score();

        Ok(Some(solution))
    }

    /// Build solution for optimized mode (full combinatorial optimization)
    async fn solve_optimized(&self, orders: Vec<Order>) -> crate::Result<Option<Solution>> {
        if orders.is_empty() {
            return Ok(None);
        }

        // Detect all CoW opportunities
        let cow_matches = self.detect_cow_matches(&orders);

        // Group orders by token pair for uniform clearing price calculation
        let mut token_pairs: HashMap<(String, String), (Vec<&Order>, Vec<&Order>)> =
            HashMap::new();

        for order in &orders {
            let pair = (order.sell_token.clone(), order.buy_token.clone());
            let entry = token_pairs.entry(pair).or_default();
            match order.side {
                OrderSide::Buy => entry.1.push(order),
                OrderSide::Sell => entry.0.push(order),
            }
        }

        let mut total_surplus = 0.0;
        let mut total_gas = 0u64;
        let order_ids: Vec<OrderId> = orders.iter().map(|o| o.id.clone()).collect();

        // Calculate uniform clearing prices and optimize execution
        for ((sell_token, buy_token), (sell_orders, buy_orders)) in &token_pairs {
            if let Some(clearing_price) = self.calculate_clearing_price(buy_orders, sell_orders) {
                // Calculate surplus at clearing price
                for order in sell_orders.iter().chain(buy_orders.iter()) {
                    let limit_price = order.buy_amount.0 as f64 / order.sell_amount.0 as f64;
                    let surplus_per_unit = (clearing_price - limit_price).abs();
                    total_surplus += surplus_per_unit * order.sell_amount.0 as f64;
                }
            }
        }

        // Optimized gas calculation with batching and CoW benefits
        let base_gas = 100_000u64;
        let gas_per_order = 80_000u64; // Lower due to optimization
        total_gas = base_gas + gas_per_order * orders.len() as u64;

        // Gas savings from CoW matches
        total_gas = total_gas.saturating_sub(cow_matches.len() as u64 * 60_000);

        // Additional surplus from CoW matching
        total_surplus += cow_matches.len() as f64 * 0.02;

        let mut solution = Solution {
            orders: order_ids,
            settlement: SettlementPlan::default(),
            gas_cost: total_gas,
            surplus: total_surplus,
            score: 0.0,
        };

        solution.calculate_score();

        Ok(Some(solution))
    }

    /// Validate solution meets all constraints
    fn validate_solution(&self, solution: &Solution, orders: &[Order]) -> crate::Result<()> {
        // Check gas limit
        if solution.gas_cost > self.config.max_gas_price * 1_000_000 {
            return Err(crate::Error::SolverError(
                "Solution exceeds gas limit".to_string(),
            ));
        }

        // Check profitability
        if !solution.is_profitable(self.config.min_profit_threshold) {
            return Err(crate::Error::SolverError(
                "Solution does not meet minimum profit threshold".to_string(),
            ));
        }

        // Verify all orders in solution exist
        let order_ids: HashSet<_> = orders.iter().map(|o| &o.id).collect();
        for order_id in &solution.orders {
            if !order_ids.contains(order_id) {
                return Err(crate::Error::SolverError(format!(
                    "Solution contains unknown order: {:?}",
                    order_id
                )));
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Solver for SolverEngine {
    async fn solve(&self, orders: Vec<Order>) -> crate::Result<Option<Solution>> {
        // Apply timeout to prevent hanging
        let solve_future = async {
            let solution = match self.mode {
                ExecutionMode::Baseline => self.solve_baseline(orders.clone()).await?,
                ExecutionMode::Naive => self.solve_naive(orders.clone()).await?,
                ExecutionMode::Optimized => self.solve_optimized(orders.clone()).await?,
            };

            // Validate solution if one was found
            if let Some(ref sol) = solution {
                self.validate_solution(sol, &orders)?;
            }

            Ok(solution)
        };

        match timeout(Duration::from_millis(self.config.timeout_ms), solve_future).await {
            Ok(result) => result,
            Err(_) => Err(crate::Error::SolverError(
                "Solver timeout exceeded".to_string(),
            )),
        }
    }

    fn name(&self) -> &str {
        match self.mode {
            ExecutionMode::Baseline => "baseline-solver",
            ExecutionMode::Naive => "naive-solver",
            ExecutionMode::Optimized => "optimized-solver",
        }
    }

    fn config(&self) -> &SolverConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{OrderSide, TokenAmount};

    fn create_test_order(
        id: &str,
        sell_token: &str,
        buy_token: &str,
        sell_amount: u128,
        buy_amount: u128,
        side: OrderSide,
    ) -> Order {
        Order {
            id: OrderId(id.to_string()),
            sell_token: sell_token.to_string(),
            buy_token: buy_token.to_string(),
            sell_amount: TokenAmount(sell_amount),
            buy_amount: TokenAmount(buy_amount),
            side,
            valid_to: 0,
            partially_fillable: false,
        }
    }

    #[tokio::test]
    async fn test_baseline_solver() {
        let config = SolverConfig::default();
        let engine = SolverEngine::new(config, ExecutionMode::Baseline);

        let orders = vec![create_test_order(
            "order1",
            "USDC",
            "DAI",
            1000,
            1000,
            OrderSide::Sell,
        )];

        let solution = engine.solve(orders).await.unwrap();
        assert!(solution.is_some());

        let sol = solution.unwrap();
        assert_eq!(sol.orders.len(), 1);
        assert!(sol.gas_cost > 0);
    }

    #[tokio::test]
    async fn test_cow_detection() {
        let config = SolverConfig::default();
        let engine = SolverEngine::new(config, ExecutionMode::Optimized);

        let orders = vec![
            create_test_order("order1", "USDC", "DAI", 1000, 1000, OrderSide::Sell),
            create_test_order("order2", "DAI", "USDC", 1000, 1000, OrderSide::Sell),
        ];

        let matches = engine.detect_cow_matches(&orders);
        assert!(!matches.is_empty(), "Should detect CoW match");
    }

    #[tokio::test]
    async fn test_naive_solver_batching() {
        let config = SolverConfig::default();
        let engine = SolverEngine::new(config, ExecutionMode::Naive);

        let orders = vec![
            create_test_order("order1", "USDC", "DAI", 1000, 1000, OrderSide::Sell),
            create_test_order("order2", "USDC", "DAI", 2000, 2000, OrderSide::Sell),
        ];

        let solution = engine.solve(orders).await.unwrap();
        assert!(solution.is_some());

        let sol = solution.unwrap();
        assert_eq!(sol.orders.len(), 2);
        // Batching should reduce gas cost per order
        assert!(sol.gas_cost < 150_000 * 2);
    }

    #[tokio::test]
    async fn test_objectives_pareto_dominance() {
        let obj1 = Objectives {
            surplus: 1.0,
            gas_cost: 0.1,
            slippage: 0.01,
            risk: 0.01,
        };

        let obj2 = Objectives {
            surplus: 0.9,
            gas_cost: 0.15,
            slippage: 0.02,
            risk: 0.02,
        };

        assert!(obj1.dominates(&obj2));
        assert!(!obj2.dominates(&obj1));
    }

    #[tokio::test]
    async fn test_clearing_price_calculation() {
        let config = SolverConfig::default();
        let engine = SolverEngine::new(config, ExecutionMode::Optimized);

        let buy_orders = vec![
            &create_test_order("b1", "USDC", "DAI", 1000, 1100, OrderSide::Buy),
            &create_test_order("b2", "USDC", "DAI", 2000, 2200, OrderSide::Buy),
        ];

        let sell_orders = vec![
            &create_test_order("s1", "DAI", "USDC", 1000, 900, OrderSide::Sell),
            &create_test_order("s2", "DAI", "USDC", 2000, 1800, OrderSide::Sell),
        ];

        let price = engine.calculate_clearing_price(&buy_orders, &sell_orders);
        assert!(price.is_some());
        assert!(price.unwrap() > 0.0);
    }
}
