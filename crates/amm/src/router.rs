//! Graph-based AMM router with multi-hop pathfinding
//!
//! Implements Dijkstra's algorithm for finding optimal routes through AMM pools,
//! with gas-adjusted pricing and configurable route constraints.

use crate::{AmmAdapter, AmmError, AmmPool, PoolMetadata, Result, Route, RouteConfig, SwapQuote};
use ethers::types::{Address, U256};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Graph-based router for finding optimal swap routes
pub struct Router {
    /// Registered AMM adapters
    adapters: Vec<Arc<dyn AmmAdapter>>,
    
    /// Pool cache: token_pair -> pools
    pool_cache: HashMap<(Address, Address), Vec<Arc<dyn AmmPool>>>,
    
    /// Token graph: token -> connected_tokens
    token_graph: HashMap<Address, HashSet<Address>>,
    
    /// Configuration
    config: RouteConfig,
}

impl Router {
    /// Create a new router with given adapters
    pub fn new(adapters: Vec<Arc<dyn AmmAdapter>>, config: RouteConfig) -> Self {
        Self {
            adapters,
            pool_cache: HashMap::new(),
            token_graph: HashMap::new(),
            config,
        }
    }
    
    /// Initialize router by loading all pools
    pub async fn initialize(&mut self) -> Result<()> {
        info!("Initializing AMM router");
        
        for adapter in &self.adapters {
            debug!("Loading pools from {}", adapter.name());
            let pools = adapter.get_pools().await?;
            
            for pool in pools {
                let tokens = pool.tokens();
                
                // Build token graph
                for (i, &token_a) in tokens.iter().enumerate() {
                    for &token_b in tokens.iter().skip(i + 1) {
                        self.token_graph
                            .entry(token_a)
                            .or_insert_with(HashSet::new)
                            .insert(token_b);
                        self.token_graph
                            .entry(token_b)
                            .or_insert_with(HashSet::new)
                            .insert(token_a);
                        
                        // Cache pool for both directions
                        self.pool_cache
                            .entry((token_a, token_b))
                            .or_insert_with(Vec::new)
                            .push(pool.clone());
                        self.pool_cache
                            .entry((token_b, token_a))
                            .or_insert_with(Vec::new)
                            .push(pool.clone());
                    }
                }
            }
        }
        
        info!(
            "Router initialized with {} token pairs",
            self.pool_cache.len()
        );
        Ok(())
    }
    
    /// Find best route for a swap
    pub async fn find_best_route(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
    ) -> Result<Route> {
        if token_in == token_out {
            return Err(AmmError::InvalidParameters(
                "Input and output tokens must be different".to_string(),
            ));
        }
        
        debug!(
            "Finding route from {:?} to {:?} for amount {}",
            token_in, token_out, amount_in
        );
        
        // Try direct route first
        if let Ok(route) = self.find_direct_route(token_in, token_out, amount_in).await {
            return Ok(route);
        }
        
        // Fall back to multi-hop if enabled
        if self.config.allow_multi_hop {
            self.find_multi_hop_route(token_in, token_out, amount_in).await
        } else {
            Err(AmmError::NoRouteFound {
                from: token_in,
                to: token_out,
            })
        }
    }
    
    /// Find direct (single-hop) route
    async fn find_direct_route(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
    ) -> Result<Route> {
        let pools = self
            .pool_cache
            .get(&(token_in, token_out))
            .ok_or_else(|| AmmError::NoRouteFound {
                from: token_in,
                to: token_out,
            })?;
        
        let mut best_route: Option<Route> = None;
        let mut best_output = U256::zero();
        
        for pool in pools {
            match pool.get_amount_out(token_in, token_out, amount_in).await {
                Ok((amount_out, gas_cost)) => {
                    let gas_adjusted = self.calculate_gas_adjusted_output(amount_out, gas_cost);
                    
                    if gas_adjusted > best_output {
                        best_output = gas_adjusted;
                        best_route = Some(Route {
                            pools: vec![pool.address()],
                            tokens: vec![token_in, token_out],
                            pool_types: vec![pool.pool_type()],
                            amount_out,
                            gas_cost,
                            price_impact_bps: self.calculate_price_impact(amount_in, amount_out),
                        });
                    }
                }
                Err(e) => {
                    warn!("Pool {:?} failed: {}", pool.address(), e);
                    continue;
                }
            }
        }
        
        best_route.ok_or_else(|| AmmError::NoRouteFound {
            from: token_in,
            to: token_out,
        })
    }
    
    /// Find multi-hop route using Dijkstra's algorithm
    async fn find_multi_hop_route(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
    ) -> Result<Route> {
        #[derive(Clone)]
        struct State {
            token: Address,
            amount: U256,
            path: Vec<Address>,
            pools: Vec<Address>,
            pool_types: Vec<crate::PoolType>,
            total_gas: u64,
        }
        
        impl Eq for State {}
        impl PartialEq for State {
            fn eq(&self, other: &Self) -> bool {
                self.amount == other.amount
            }
        }
        impl Ord for State {
            fn cmp(&self, other: &Self) -> Ordering {
                self.amount.cmp(&other.amount)
            }
        }
        impl PartialOrd for State {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }
        
        let mut heap = BinaryHeap::new();
        let mut visited = HashSet::new();
        
        heap.push(State {
            token: token_in,
            amount: amount_in,
            path: vec![token_in],
            pools: vec![],
            pool_types: vec![],
            total_gas: 0,
        });
        
        while let Some(state) = heap.pop() {
            if state.token == token_out {
                return Ok(Route {
                    pools: state.pools,
                    tokens: state.path,
                    pool_types: state.pool_types,
                    amount_out: state.amount,
                    gas_cost: state.total_gas,
                    price_impact_bps: self.calculate_price_impact(amount_in, state.amount),
                });
            }
            
            if state.path.len() > self.config.max_hops {
                continue;
            }
            
            if !visited.insert((state.token, state.path.len())) {
                continue;
            }
            
            let neighbors = match self.token_graph.get(&state.token) {
                Some(n) => n,
                None => continue,
            };
            
            for &next_token in neighbors {
                if state.path.contains(&next_token) {
                    continue; // Avoid cycles
                }
                
                if let Some(pools) = self.pool_cache.get(&(state.token, next_token)) {
                    for pool in pools {
                        match pool
                            .get_amount_out(state.token, next_token, state.amount)
                            .await
                        {
                            Ok((amount_out, gas_cost)) => {
                                let mut new_path = state.path.clone();
                                new_path.push(next_token);
                                
                                let mut new_pools = state.pools.clone();
                                new_pools.push(pool.address());
                                
                                let mut new_pool_types = state.pool_types.clone();
                                new_pool_types.push(pool.pool_type());
                                
                                heap.push(State {
                                    token: next_token,
                                    amount: amount_out,
                                    path: new_path,
                                    pools: new_pools,
                                    pool_types: new_pool_types,
                                    total_gas: state.total_gas + gas_cost,
                                });
                            }
                            Err(_) => continue,
                        }
                    }
                }
            }
        }
        
        Err(AmmError::NoRouteFound {
            from: token_in,
            to: token_out,
        })
    }
    
    /// Get quote for a swap
    pub async fn get_quote(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
    ) -> Result<SwapQuote> {
        let route = self.find_best_route(token_in, token_out, amount_in).await?;
        
        let min_amount_out = route.amount_out
            * U256::from(10000 - self.config.slippage_bps)
            / U256::from(10000);
        
        Ok(SwapQuote {
            token_in,
            token_out,
            amount_in,
            amount_out: route.amount_out,
            route,
            min_amount_out,
            slippage_bps: self.config.slippage_bps,
        })
    }
    
    /// Calculate gas-adjusted output
    fn calculate_gas_adjusted_output(&self, amount_out: U256, gas_cost: u64) -> U256 {
        let gas_cost_wei = U256::from(gas_cost) * self.config.gas_price;
        if amount_out > gas_cost_wei {
            amount_out - gas_cost_wei
        } else {
            U256::zero()
        }
    }
    
    /// Calculate price impact in basis points
    fn calculate_price_impact(&self, amount_in: U256, amount_out: U256) -> u64 {
        if amount_in.is_zero() || amount_out.is_zero() {
            return 0;
        }
        
        // Simplified price impact calculation
        // In production, this should use oracle prices
        let ratio = amount_out * U256::from(10000) / amount_in;
        let expected_ratio = U256::from(10000);
        
        if ratio < expected_ratio {
            ((expected_ratio - ratio) * U256::from(10000) / expected_ratio)
                .as_u64()
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_creation() {
        let config = RouteConfig::default();
        let router = Router::new(vec![], config);
        assert_eq!(router.adapters.len(), 0);
    }
}
