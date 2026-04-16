use alloy_primitives::{Address, B256};
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::proc_macros::rpc;
use reth_transaction_pool::TransactionPool;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// anvil_* RPC namespace.
///
/// Mirrors the trait defined in `reth-rpc-api`, which does not publicly export
/// `AnvilApiServer` in its `servers` module.
#[rpc(server, namespace = "anvil")]
pub trait AnvilApi {
    #[method(name = "impersonateAccount")]
    async fn anvil_impersonate_account(&self, address: Address) -> RpcResult<()>;

    #[method(name = "stopImpersonatingAccount")]
    async fn anvil_stop_impersonating_account(&self, address: Address) -> RpcResult<()>;

    #[method(name = "autoImpersonateAccount")]
    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> RpcResult<()>;

    #[method(name = "getAutomine")]
    async fn anvil_get_automine(&self) -> RpcResult<bool>;

    #[method(name = "dropTransaction")]
    async fn anvil_drop_transaction(&self, tx_hash: B256) -> RpcResult<Option<B256>>;

    #[method(name = "dropAllTransactions")]
    async fn anvil_drop_all_transactions(&self) -> RpcResult<()>;

    #[method(name = "removePoolTransactions")]
    async fn anvil_remove_pool_transactions(&self, address: Address) -> RpcResult<()>;

    #[method(name = "setLoggingEnabled")]
    async fn anvil_set_logging_enabled(&self, enabled: bool) -> RpcResult<()>;
}

/// Shared state for impersonated accounts.
#[derive(Debug, Default)]
struct AnvilState {
    impersonated: HashSet<Address>,
    auto_impersonate: bool,
}

/// Implementation of the `anvil_*` RPC namespace.
#[derive(Debug, Clone)]
pub struct AnvilRpc<Pool> {
    state: Arc<RwLock<AnvilState>>,
    pool: Pool,
}

impl<Pool> AnvilRpc<Pool> {
    pub fn new(pool: Pool) -> Self {
        Self {
            state: Arc::new(RwLock::new(AnvilState::default())),
            pool,
        }
    }
}

#[async_trait]
impl<Pool> AnvilApiServer for AnvilRpc<Pool>
where
    Pool: TransactionPool + Send + Sync + 'static,
{
    async fn anvil_impersonate_account(&self, address: Address) -> RpcResult<()> {
        self.state.write().unwrap().impersonated.insert(address);
        Ok(())
    }

    async fn anvil_stop_impersonating_account(&self, address: Address) -> RpcResult<()> {
        self.state.write().unwrap().impersonated.remove(&address);
        Ok(())
    }

    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> RpcResult<()> {
        self.state.write().unwrap().auto_impersonate = enabled;
        Ok(())
    }

    async fn anvil_get_automine(&self) -> RpcResult<bool> {
        Ok(true)
    }

    async fn anvil_drop_transaction(&self, tx_hash: B256) -> RpcResult<Option<B256>> {
        Ok(self.pool.remove_transaction(tx_hash).map(|_| tx_hash))
    }

    async fn anvil_drop_all_transactions(&self) -> RpcResult<()> {
        let hashes = self.pool.all_transaction_hashes();
        if !hashes.is_empty() {
            self.pool.remove_transactions(hashes);
        }
        Ok(())
    }

    async fn anvil_remove_pool_transactions(&self, address: Address) -> RpcResult<()> {
        self.pool.remove_transactions_by_sender(address);
        Ok(())
    }

    async fn anvil_set_logging_enabled(&self, _enabled: bool) -> RpcResult<()> {
        Ok(())
    }
}
