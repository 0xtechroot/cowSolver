use serde::{Deserialize, Serialize};
use ethers::types::{Address, U256};
use super::chains::ChainId;

/// Represents a token on a specific chain
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Token {
    /// Token contract address
    pub address: Address,
    
    /// Chain where token exists
    pub chain_id: ChainId,
    
    /// Token symbol (e.g., "USDC", "ETH")
    pub symbol: String,
    
    /// Token name
    pub name: String,
    
    /// Number of decimals
    pub decimals: u8,
}

/// Token amount with decimal awareness
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenAmount {
    /// Raw amount in smallest unit
    pub raw: U256,
    
    /// Number of decimals
    pub decimals: u8,
}

impl TokenAmount {
    /// Creates a new token amount
    pub fn new(raw: U256, decimals: u8) -> Self {
        Self { raw, decimals }
    }
    
    /// Creates token amount from human-readable value
    pub fn from_decimal(value: f64, decimals: u8) -> Self {
        let multiplier = 10_u128.pow(decimals as u32);
        let raw = U256::from((value * multiplier as f64) as u128);
        Self { raw, decimals }
    }
    
    /// Converts to human-readable decimal value
    pub fn to_decimal(&self) -> f64 {
        let divisor = 10_u128.pow(self.decimals as u32) as f64;
        self.raw.as_u128() as f64 / divisor
    }
    
    /// Checks if amount is zero
    pub fn is_zero(&self) -> bool {
        self.raw.is_zero()
    }
    
    /// Adds two token amounts (must have same decimals)
    pub fn checked_add(&self, other: &TokenAmount) -> Option<TokenAmount> {
        if self.decimals != other.decimals {
            return None;
        }
        
        self.raw.checked_add(other.raw).map(|raw| TokenAmount {
            raw,
            decimals: self.decimals,
        })
    }
    
    /// Subtracts two token amounts (must have same decimals)
    pub fn checked_sub(&self, other: &TokenAmount) -> Option<TokenAmount> {
        if self.decimals != other.decimals {
            return None;
        }
        
        self.raw.checked_sub(other.raw).map(|raw| TokenAmount {
            raw,
            decimals: self.decimals,
        })
    }
    
    /// Multiplies amount by a scalar
    pub fn checked_mul(&self, scalar: u128) -> Option<TokenAmount> {
        self.raw.checked_mul(U256::from(scalar)).map(|raw| TokenAmount {
            raw,
            decimals: self.decimals,
        })
    }
    
    /// Divides amount by a scalar
    pub fn checked_div(&self, scalar: u128) -> Option<TokenAmount> {
        if scalar == 0 {
            return None;
        }
        
        self.raw.checked_div(U256::from(scalar)).map(|raw| TokenAmount {
            raw,
            decimals: self.decimals,
        })
    }
}

impl Token {
    /// Creates a new token
    pub fn new(
        address: Address,
        chain_id: ChainId,
        symbol: String,
        name: String,
        decimals: u8,
    ) -> Self {
        Self {
            address,
            chain_id,
            symbol,
            name,
            decimals,
        }
    }
    
    /// Creates a token amount for this token
    pub fn amount(&self, raw: U256) -> TokenAmount {
        TokenAmount::new(raw, self.decimals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_token_amount_from_decimal() {
        let amount = TokenAmount::from_decimal(1.5, 18);
        assert_eq!(amount.to_decimal(), 1.5);
    }
    
    #[test]
    fn test_token_amount_addition() {
        let a = TokenAmount::new(U256::from(100), 18);
        let b = TokenAmount::new(U256::from(50), 18);
        let result = a.checked_add(&b).unwrap();
        assert_eq!(result.raw, U256::from(150));
    }
    
    #[test]
    fn test_token_amount_subtraction() {
        let a = TokenAmount::new(U256::from(100), 18);
        let b = TokenAmount::new(U256::from(50), 18);
        let result = a.checked_sub(&b).unwrap();
        assert_eq!(result.raw, U256::from(50));
    }
    
    #[test]
    fn test_token_amount_different_decimals() {
        let a = TokenAmount::new(U256::from(100), 18);
        let b = TokenAmount::new(U256::from(50), 6);
        assert!(a.checked_add(&b).is_none());
    }
    
    #[test]
    fn test_token_amount_multiplication() {
        let amount = TokenAmount::new(U256::from(100), 18);
        let result = amount.checked_mul(2).unwrap();
        assert_eq!(result.raw, U256::from(200));
    }
    
    #[test]
    fn test_token_amount_division() {
        let amount = TokenAmount::new(U256::from(100), 18);
        let result = amount.checked_div(2).unwrap();
        assert_eq!(result.raw, U256::from(50));
    }

    #[test]
    fn test_is_zero_returns_true_for_zero_and_false_for_nonzero() {
        let zero = TokenAmount::new(U256::from(0), 8);
        let non_zero = TokenAmount::new(U256::from(1), 8);
        assert!(zero.is_zero());
        assert!(!non_zero.is_zero());
    }

    #[test]
    fn test_checked_divide_by_zero_returns_none() {
        let amount = TokenAmount::new(U256::from(10), 8);
        assert!(amount.checked_div(0).is_none());
    }

    #[test]
    fn test_checked_sub_underflow_returns_none() {
        let small = TokenAmount::new(U256::from(5), 6);
        let big = TokenAmount::new(U256::from(10), 6);
        assert!(small.checked_sub(&big).is_none());
    }

    #[test]
    fn test_checked_mul_overflow_returns_none() {
        // construct a value that will overflow when multiplied by 2
        let half_plus_one = (U256::MAX / U256::from(2u8)) + U256::from(1u8);
        let amount = TokenAmount::new(half_plus_one, 18);
        // multiplying by 2 should overflow
        assert!(amount.checked_mul(2).is_none());
    }

    #[test]
    fn test_checked_add_overflow_returns_none() {
        let almost_max = U256::MAX - U256::from(1u8);
        let a = TokenAmount::new(almost_max, 12);
        let b = TokenAmount::new(U256::from(2u8), 12);
        assert!(a.checked_add(&b).is_none());
    }

    #[test]
    fn test_division_floor_behavior() {
        let amount = TokenAmount::new(U256::from(3), 0);
        // 3 / 2 == floor(1.5) == 1
        let res = amount.checked_div(2).unwrap();
        assert_eq!(res.raw, U256::from(1));
    }

    #[test]
    fn test_from_decimal_truncation_for_zero_decimals() {
        // from_decimal truncates toward zero when decimals == 0
        let amt = TokenAmount::from_decimal(1.9999, 0);
        assert_eq!(amt.raw, U256::from(1u128));
    }

    #[test]
    fn test_to_decimal_various_decimals() {
        let a = TokenAmount::new(U256::from(123456u128), 6);
        let d = a.to_decimal();
        // 123456 with 6 decimals -> 0.123456
        let expected = 0.123456_f64;
        let diff = (d - expected).abs();
        assert!(diff < 1e-12, "diff {} >= epsilon", diff);
    }

    #[test]
    fn test_roundtrip_mul_div_without_overflow() {
        // For small values, (a * s) / s == a
        for raw in 0u128..200 {
            let a = TokenAmount::new(U256::from(raw), 8);
            for s in 1u128..10u128 {
                let mul = a.checked_mul(s).expect("should not overflow for these ranges");
                let div = mul.checked_div(s).expect("division by non-zero");
                assert_eq!(div.raw, a.raw, "raw:{} scalar:{}", raw, s);
                assert_eq!(div.decimals, a.decimals);
            }
        }
    }

    #[test]
    fn test_from_and_to_decimal_roundtrip() {
        // test a variety of decimal precisions and values
        let cases = [
            (1.2345_f64, 6u8),
            (0.0000012345_f64, 18u8),
            (42.0_f64, 0u8),
            (99999.9999_f64, 4u8),
        ];
        for (val, decimals) in &cases {
            let ta = TokenAmount::from_decimal(*val, *decimals);
            let back = ta.to_decimal();
            // Allow small error due to truncation when converting to raw
            let diff = (back - val).abs();
            assert!(diff <= (1.0 / 10_f64.powi(*decimals as i32)), "val:{} decimals:{} diff:{}", val, decimals, diff);
        }
    }

    // A simple property-like deterministic test (no external crates) that checks arithmetic associativity:
    // (a + b) - b == a when decimals match and no overflow/underflow occurs.
    #[test]
    fn test_add_sub_property_small_range() {
        for a_raw in 0u128..500 {
            for b_raw in 0u128..500 {
                let a = TokenAmount::new(U256::from(a_raw), 9);
                let b = TokenAmount::new(U256::from(b_raw), 9);
                // attempt add
                if let Some(sum) = a.checked_add(&b) {
                    // then subtract b from sum
                    if let Some(back) = sum.checked_sub(&b) {
                        assert_eq!(back.raw, a.raw);
                        assert_eq!(back.decimals, a.decimals);
                    }
                }
            }
        }
    }
}
