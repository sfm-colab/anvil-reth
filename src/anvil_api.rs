use crate::impersonation::ImpersonationState;
use alloy_primitives::{Address, B256};
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::proc_macros::rpc;
use reth_transaction_pool::TransactionPool;

/// anvil_* RPC namespace.
///
/// Mirrors the trait defined in `reth-rpc-api`, which does not publicly export
/// `AnvilApiServer` in its `servers` module.
#[rpc(server, namespace = "anvil")]
pub trait AnvilApi {
    #[method(name = "impersonateAccount", aliases = ["hardhat_impersonateAccount"])]
    async fn anvil_impersonate_account(&self, address: Address) -> RpcResult<()>;

    #[method(
        name = "stopImpersonatingAccount",
        aliases = ["hardhat_stopImpersonatingAccount"]
    )]
    async fn anvil_stop_impersonating_account(&self, address: Address) -> RpcResult<()>;

    #[method(
        name = "autoImpersonateAccount",
        aliases = ["hardhat_autoImpersonateAccount"]
    )]
    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> RpcResult<()>;

    #[method(name = "getAutomine", aliases = ["hardhat_getAutomine"])]
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

/// Implementation of the `anvil_*` RPC namespace.
#[derive(Debug, Clone)]
pub struct AnvilRpc<Pool> {
    state: ImpersonationState,
    pool: Pool,
}

impl<Pool> AnvilRpc<Pool> {
    pub fn new(state: ImpersonationState, pool: Pool) -> Self {
        Self { state, pool }
    }
}

#[async_trait]
impl<Pool> AnvilApiServer for AnvilRpc<Pool>
where
    Pool: TransactionPool + Send + Sync + 'static,
{
    async fn anvil_impersonate_account(&self, address: Address) -> RpcResult<()> {
        self.state.impersonate(address);
        Ok(())
    }

    async fn anvil_stop_impersonating_account(&self, address: Address) -> RpcResult<()> {
        self.state.stop_impersonating(address);
        Ok(())
    }

    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> RpcResult<()> {
        self.state.set_auto_impersonate(enabled);
        Ok(())
    }

    async fn anvil_get_automine(&self) -> RpcResult<bool> {
        Ok(true)
    }

    async fn anvil_drop_transaction(&self, tx_hash: B256) -> RpcResult<Option<B256>> {
        Ok(self.pool.remove_transaction(tx_hash).map(|_| {
            self.state.forget_tx_sender(&tx_hash);
            tx_hash
        }))
    }

    async fn anvil_drop_all_transactions(&self) -> RpcResult<()> {
        let hashes = self.pool.all_transaction_hashes();
        if !hashes.is_empty() {
            self.pool.remove_transactions(hashes.clone());
            self.state.forget_tx_senders(hashes);
        }
        Ok(())
    }

    async fn anvil_remove_pool_transactions(&self, address: Address) -> RpcResult<()> {
        let removed = self.pool.remove_transactions_by_sender(address);
        self.state
            .forget_tx_senders(removed.into_iter().map(|tx| *tx.hash()));
        Ok(())
    }

    async fn anvil_set_logging_enabled(&self, _enabled: bool) -> RpcResult<()> {
        Ok(())
    }
}
