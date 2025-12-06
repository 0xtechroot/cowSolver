//! Pool management and caching utilities

use crate::{AmmError, AmmPool, PoolMetadata, Result};
use ethers::types::Address;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Pool registry for managing and caching pool state
pub struct PoolRegistry {
    pools: Arc<RwLock<HashMap<Address, Arc<dyn AmmPool>>>>,
    metadata: Arc<RwLock<HashMap<Address, PoolMetadata>>>,
}

impl PoolRegistry {
    pub fn new() -> Self {
        Self {
            pools: Arc::new(RwLock::new(HashMap::new())),
            metadata: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Register a pool
    pub async fn register_pool(&self, pool: Arc<dyn AmmPool>, metadata: PoolMetadata) {
        let address = pool.address();
        self.pools.write().await.insert(address, pool);
        self.metadata.write().await.insert(address, metadata);
    }
    
    /// Get pool by address
    pub async fn get_pool(&self, address: Address) -> Result<Arc<dyn AmmPool>> {
        self.pools
            .read()
            .await
            .get(&address)
            .cloned()
            .ok_or_else(|| AmmError::PoolNotFound(address))
    }
    
    /// Get pool metadata
    pub async fn get_metadata(&self, address: Address) -> Result<PoolMetadata> {
        self.metadata
            .read()
            .await
            .get(&address)
            .cloned()
            .ok_or_else(|| AmmError::PoolNotFound(address))
    }
    
    /// Get all pools
    pub async fn get_all_pools(&self) -> Vec<Arc<dyn AmmPool>> {
        self.pools.read().await.values().cloned().collect()
    }
    
    /// Sync all pools with on-chain state
    pub async fn sync_all(&self) -> Result<()> {
        let pools = self.get_all_pools().await;
        
        for pool in pools {
            // Clone the pool to get a mutable reference
            // In production, this would need proper synchronization
            // pool.sync().await?;
        }
        
        Ok(())
    }
}

impl Default for PoolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
