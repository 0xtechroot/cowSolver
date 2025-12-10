use serde::{Deserialize, Serialize};

/// Supported blockchain networks
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ChainId {
    Ethereum = 1,
    Optimism = 10,
    BinanceSmartChain = 56,
    Polygon = 137,
    Base = 8453,
    Arbitrum = 42161,
    Avalanche = 43114,
}

impl ChainId {
    /// Returns chain name
    pub fn name(&self) -> &'static str {
        match self {
            ChainId::Ethereum => "Ethereum",
            ChainId::Optimism => "Optimism",
            ChainId::BinanceSmartChain => "Binance Smart Chain",
            ChainId::Polygon => "Polygon",
            ChainId::Base => "Base",
            ChainId::Arbitrum => "Arbitrum",
            ChainId::Avalanche => "Avalanche",
        }
    }
    
    /// Returns native token symbol
    pub fn native_token(&self) -> &'static str {
        match self {
            ChainId::Ethereum => "ETH",
            ChainId::Optimism => "ETH",
            ChainId::BinanceSmartChain => "BNB",
            ChainId::Polygon => "MATIC",
            ChainId::Base => "ETH",
            ChainId::Arbitrum => "ETH",
            ChainId::Avalanche => "AVAX",
        }
    }
    
    /// Checks if chain is EVM compatible
    pub fn is_evm(&self) -> bool {
        true // All currently supported chains are EVM
    }
    
    /// Returns typical block time in seconds
    pub fn block_time(&self) -> u64 {
        match self {
            ChainId::Ethereum => 12,
            ChainId::Optimism => 2,
            ChainId::BinanceSmartChain => 3,
            ChainId::Polygon => 2,
            ChainId::Base => 2,
            ChainId::Arbitrum => 1,
            ChainId::Avalanche => 2,
        }
    }
    
    /// Returns chain ID as u64
    pub fn as_u64(&self) -> u64 {
        *self as u64
    }
    
    /// Creates ChainId from u64
    pub fn from_u64(id: u64) -> Option<Self> {
        match id {
            1 => Some(ChainId::Ethereum),
            10 => Some(ChainId::Optimism),
            56 => Some(ChainId::BinanceSmartChain),
            137 => Some(ChainId::Polygon),
            8453 => Some(ChainId::Base),
            42161 => Some(ChainId::Arbitrum),
            43114 => Some(ChainId::Avalanche),
            _ => None,
        }
    }
}

/// Supported chain configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportedChain {
    pub chain_id: ChainId,
    pub rpc_url: String,
    pub explorer_url: String,
    pub cow_settlement_address: Option<String>,
}

impl SupportedChain {
    pub fn new(
        chain_id: ChainId,
        rpc_url: String,
        explorer_url: String,
        cow_settlement_address: Option<String>,
    ) -> Self {
        Self {
            chain_id,
            rpc_url,
            explorer_url,
            cow_settlement_address,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_chain_id_conversion() {
        assert_eq!(ChainId::from_u64(1), Some(ChainId::Ethereum));
        assert_eq!(ChainId::from_u64(137), Some(ChainId::Polygon));
        assert_eq!(ChainId::from_u64(999), None);
    }
    
    #[test]
    fn test_chain_properties() {
        assert_eq!(ChainId::Ethereum.name(), "Ethereum");
        assert_eq!(ChainId::Ethereum.native_token(), "ETH");
        assert_eq!(ChainId::Polygon.native_token(), "MATIC");
        assert!(ChainId::Ethereum.is_evm());
    }
    
    #[test]
    fn test_block_times() {
        assert_eq!(ChainId::Ethereum.block_time(), 12);
        assert_eq!(ChainId::Arbitrum.block_time(), 1);
    }
}

#[cfg(test)]
mod extra_chains_tests {
    use super::*;
    use serde_json;

    #[test]
    fn chainid_as_u64_and_from_u64_roundtrip_all_variants() {
        let all = [
            ChainId::Ethereum,
            ChainId::Optimism,
            ChainId::BinanceSmartChain,
            ChainId::Polygon,
            ChainId::Base,
            ChainId::Arbitrum,
            ChainId::Avalanche,
        ];
        for &c in &all {
            let id = c.as_u64();
            let from = ChainId::from_u64(id);
            assert_eq!(from, Some(c));
        }
    }

    #[test]
    fn chain_native_token_and_name_consistency() {
        assert_eq!(ChainId::Ethereum.native_token(), "ETH");
        assert_eq!(ChainId::Ethereum.name(), "Ethereum");
        assert_eq!(ChainId::Avalanche.native_token(), "AVAX");
    }

    #[test]
    fn supported_chain_new_and_fields() {
        let sc = SupportedChain::new(
            ChainId::Base,
            "https://rpc.base".to_string(),
            "https://explorer.base".to_string(),
            Some("0xsettle".to_string()),
        );
        assert_eq!(sc.chain_id, ChainId::Base);
        assert_eq!(sc.rpc_url, "https://rpc.base");
        assert_eq!(sc.explorer_url, "https://explorer.base");
        assert_eq!(sc.cow_settlement_address.as_deref(), Some("0xsettle"));
    }

    #[test]
    fn supported_chain_serde_roundtrip() {
        let sc = SupportedChain::new(
            ChainId::Polygon,
            "https://rpc.poly".to_string(),
            "https://explorer.poly".to_string(),
            None,
        );
        let s = serde_json::to_string(&sc).expect("serialize");
        let back: SupportedChain = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back.chain_id, sc.chain_id);
        assert_eq!(back.rpc_url, sc.rpc_url);
    }
}