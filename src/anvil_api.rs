use crate::impersonation::ImpersonationState;
use crate::mining::MiningController;
use alloy_primitives::{Address, B256, U256};
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::{
    error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    ErrorObjectOwned,
};
use reth_storage_api::BlockNumReader;
use reth_transaction_pool::TransactionPool;
use std::time::Duration;
use tokio::time::sleep;

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

    #[method(name = "getIntervalMining")]
    async fn anvil_get_interval_mining(&self) -> RpcResult<Option<u64>>;

    #[method(name = "setAutomine", aliases = ["evm_setAutomine"])]
    async fn anvil_set_automine(&self, enabled: bool) -> RpcResult<()>;

    #[method(name = "setIntervalMining", aliases = ["evm_setIntervalMining"])]
    async fn anvil_set_interval_mining(&self, interval: u64) -> RpcResult<()>;

    #[method(name = "mine", aliases = ["hardhat_mine"])]
    async fn anvil_mine(&self, num_blocks: Option<U256>, interval: Option<U256>) -> RpcResult<()>;

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
pub struct AnvilRpc<Pool, Provider> {
    state: ImpersonationState,
    mining: MiningController,
    pool: Pool,
    provider: Provider,
}

impl<Pool, Provider> AnvilRpc<Pool, Provider> {
    pub fn new(
        state: ImpersonationState,
        mining: MiningController,
        pool: Pool,
        provider: Provider,
    ) -> Self {
        Self {
            state,
            mining,
            pool,
            provider,
        }
    }
}

fn internal_error(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, message.into(), None::<()>)
}

fn invalid_params(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, message.into(), None::<()>)
}

impl<Pool, Provider> AnvilRpc<Pool, Provider>
where
    Provider: BlockNumReader,
{
    async fn wait_for_block_number(&self, expected: u64) -> RpcResult<()> {
        for _ in 0..100 {
            let current = self.provider.best_block_number().map_err(|error| {
                internal_error(format!("failed to read latest block number: {error}"))
            })?;
            if current >= expected {
                return Ok(());
            }
            sleep(Duration::from_millis(100)).await;
        }

        Err(internal_error(format!(
            "timed out waiting for block {expected}"
        )))
    }
}

#[async_trait]
impl<Pool, Provider> AnvilApiServer for AnvilRpc<Pool, Provider>
where
    Pool: TransactionPool + Send + Sync + 'static,
    Provider: BlockNumReader + Send + Sync + 'static,
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
        Ok(self.mining.is_automine())
    }

    async fn anvil_get_interval_mining(&self) -> RpcResult<Option<u64>> {
        Ok(self.mining.interval_mining())
    }

    async fn anvil_set_automine(&self, enabled: bool) -> RpcResult<()> {
        self.mining.set_automine(enabled);
        if enabled && self.pool.pending_and_queued_txn_count().0 > 0 {
            self.mining.trigger();
        }
        Ok(())
    }

    async fn anvil_set_interval_mining(&self, interval: u64) -> RpcResult<()> {
        self.mining.set_interval_mining(interval);
        Ok(())
    }

    async fn anvil_mine(&self, num_blocks: Option<U256>, interval: Option<U256>) -> RpcResult<()> {
        if interval.is_some_and(|interval| !interval.is_zero()) {
            return Err(invalid_params("anvil_mine interval is not supported yet"));
        }

        let blocks = num_blocks.unwrap_or(U256::from(1)).to::<u64>();
        if blocks == 0 {
            return Ok(());
        }

        let start = self.provider.best_block_number().map_err(|error| {
            internal_error(format!("failed to read latest block number: {error}"))
        })?;

        for _ in 0..blocks {
            self.mining.trigger();
        }

        self.wait_for_block_number(start.saturating_add(blocks))
            .await
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
