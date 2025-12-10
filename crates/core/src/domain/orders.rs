use serde::{Deserialize, Serialize};
use ethers::types::{Address, U256};
use super::tokens::TokenAmount;
use super::chains::ChainId;

/// Represents a CoW Protocol order
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Order {
    /// Unique order identifier
    pub id: OrderId,
    
    /// Order creator address
    pub owner: Address,
    
    /// Token to sell
    pub sell_token: Address,
    
    /// Token to buy
    pub buy_token: Address,
    
    /// Amount of sell token
    pub sell_amount: U256,
    
    /// Amount of buy token
    pub buy_amount: U256,
    
    /// Order validity timestamp
    pub valid_to: u32,
    
    /// Fee amount in sell token
    pub fee_amount: U256,
    
    /// Order type
    pub kind: OrderType,
    
    /// Partially fillable flag
    pub partially_fillable: bool,
    
    /// Order status
    pub status: OrderStatus,
    
    /// Source chain for cross-chain orders
    pub source_chain: Option<ChainId>,
    
    /// Destination chain for cross-chain orders
    pub destination_chain: Option<ChainId>,
    
    /// Bridge provider for cross-chain orders
    pub bridge_provider: Option<String>,
}

/// Order unique identifier
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct OrderId(pub [u8; 32]);

/// Order execution type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderType {
    /// Buy order (buy exact amount)
    Buy,
    /// Sell order (sell exact amount)
    Sell,
}

/// Order lifecycle status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderStatus {
    /// Order is open and can be filled
    Open,
    /// Order is being processed
    Pending,
    /// Order has been filled
    Filled,
    /// Order has been partially filled
    PartiallyFilled,
    /// Order has been cancelled
    Cancelled,
    /// Order has expired
    Expired,
}

impl Order {
    /// Validates order parameters
    pub fn validate(&self) -> Result<(), String> {
        if self.sell_amount.is_zero() {
            return Err("Sell amount must be greater than zero".to_string());
        }
        
        if self.buy_amount.is_zero() {
            return Err("Buy amount must be greater than zero".to_string());
        }
        
        if self.sell_token == self.buy_token {
            return Err("Sell and buy tokens must be different".to_string());
        }
        
        if self.valid_to == 0 {
            return Err("Valid_to timestamp must be set".to_string());
        }
        
        // Cross-chain validation
        if self.is_cross_chain() {
            if self.source_chain.is_none() || self.destination_chain.is_none() {
                return Err("Cross-chain orders must specify both source and destination chains".to_string());
            }
            
            if self.bridge_provider.is_none() {
                return Err("Cross-chain orders must specify a bridge provider".to_string());
            }
        }
        
        Ok(())
    }
    
    /// Checks if order is cross-chain
    pub fn is_cross_chain(&self) -> bool {
        self.source_chain.is_some() && self.destination_chain.is_some()
    }
    
    /// Checks if order is expired
    pub fn is_expired(&self, current_time: u32) -> bool {
        current_time > self.valid_to
    }
    
    /// Calculates limit price (buy_amount / sell_amount)
    pub fn limit_price(&self) -> f64 {
        if self.sell_amount.is_zero() {
            return 0.0;
        }
        
        let buy = self.buy_amount.as_u128() as f64;
        let sell = self.sell_amount.as_u128() as f64;
        buy / sell
    }
    
    /// Checks if order can be filled at given price
    pub fn can_fill_at_price(&self, price: f64) -> bool {
        match self.kind {
            OrderType::Buy => price <= self.limit_price(),
            OrderType::Sell => price >= self.limit_price(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn create_test_order() -> Order {
        Order {
            id: OrderId([0u8; 32]),
            owner: Address::zero(),
            sell_token: Address::from_low_u64_be(1),
            buy_token: Address::from_low_u64_be(2),
            sell_amount: U256::from(1000),
            buy_amount: U256::from(2000),
            valid_to: 9999999999,
            fee_amount: U256::from(10),
            kind: OrderType::Sell,
            partially_fillable: false,
            status: OrderStatus::Open,
            source_chain: None,
            destination_chain: None,
            bridge_provider: None,
        }
    }
    
    #[test]
    fn test_order_validation_success() {
        let order = create_test_order();
        assert!(order.validate().is_ok());
    }
    
    #[test]
    fn test_order_validation_zero_sell_amount() {
        let mut order = create_test_order();
        order.sell_amount = U256::zero();
        assert!(order.validate().is_err());
    }
    
    #[test]
    fn test_order_validation_same_tokens() {
        let mut order = create_test_order();
        order.buy_token = order.sell_token;
        assert!(order.validate().is_err());
    }
    
    #[test]
    fn test_limit_price_calculation() {
        let order = create_test_order();
        assert_eq!(order.limit_price(), 2.0);
    }
    
    #[test]
    fn test_is_expired() {
        let order = create_test_order();
        assert!(!order.is_expired(1000));
        assert!(order.is_expired(10000000000));
    }
    
    #[test]
    fn test_cross_chain_validation() {
        let mut order = create_test_order();
        order.source_chain = Some(ChainId::Ethereum);
        order.destination_chain = Some(ChainId::Arbitrum);
        assert!(order.validate().is_err()); // Missing bridge provider
        
        order.bridge_provider = Some("Across".to_string());
        assert!(order.validate().is_ok());
    }
}

#[cfg(test)]
mod extra_orders_tests {
    use super::*;
    use serde_json;

    fn base_order() -> Order {
        Order {
            id: OrderId([1u8; 32]),
            owner: Address::zero(),
            sell_token: Address::from_low_u64_be(0x1),
            buy_token: Address::from_low_u64_be(0x2),
            sell_amount: U256::from(100u64),
            buy_amount: U256::from(200u64),
            valid_to: 1_640_000_000u32,
            fee_amount: U256::from(1u64),
            kind: OrderType::Sell,
            partially_fillable: true,
            status: OrderStatus::Open,
            source_chain: None,
            destination_chain: None,
            bridge_provider: None,
        }
    }

    #[test]
    fn limit_price_returns_zero_when_sell_amount_zero() {
        let mut o = base_order();
        o.sell_amount = U256::zero();
        o.buy_amount = U256::from(1000u64);
        assert_eq!(o.limit_price(), 0.0);
    }

    #[test]
    fn can_fill_at_price_edge_cases_buy_and_sell() {
        let mut buy_order = base_order();
        buy_order.kind = OrderType::Buy;
        buy_order.sell_amount = U256::from(2u64);
        buy_order.buy_amount = U256::from(5u64); // limit_price = 2.5
        assert!(buy_order.can_fill_at_price(2.5)); // equal is allowed
        assert!(buy_order.can_fill_at_price(2.0));
        assert!(!buy_order.can_fill_at_price(3.0));

        let mut sell_order = base_order();
        sell_order.kind = OrderType::Sell;
        sell_order.sell_amount = U256::from(4u64);
        sell_order.buy_amount = U256::from(10u64); // limit_price = 2.5
        assert!(sell_order.can_fill_at_price(2.5)); // equal is allowed
        assert!(sell_order.can_fill_at_price(3.0));
        assert!(!sell_order.can_fill_at_price(2.0));
    }

    #[test]
    fn is_cross_chain_false_when_one_chain_missing() {
        let mut o = base_order();
        o.source_chain = Some(ChainId::Ethereum);
        o.destination_chain = None;
        assert!(!o.is_cross_chain());

        o.source_chain = None;
        o.destination_chain = Some(ChainId::Arbitrum);
        assert!(!o.is_cross_chain());
    }

    #[test]
    fn validate_rejects_valid_to_zero() {
        let mut o = base_order();
        o.valid_to = 0;
        let res = o.validate();
        assert!(res.is_err());
        let msg = res.err().unwrap();
        assert!(msg.contains("Valid_to"));
    }

    #[test]
    fn order_serde_roundtrip() {
        let mut o = base_order();
        o.source_chain = Some(ChainId::Optimism);
        o.destination_chain = Some(ChainId::Arbitrum);
        o.bridge_provider = Some("TestBridge".to_string());
        let s = serde_json::to_string(&o).expect("serialize");
        let back: Order = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back.id.0, o.id.0);
        assert_eq!(back.source_chain, o.source_chain);
        assert_eq!(back.bridge_provider, o.bridge_provider);
    }
}