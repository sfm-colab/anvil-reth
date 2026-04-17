use crate::block_source::BlockSource;
use crate::impersonation::ImpersonationState;
use crate::mining::MiningController;
use crate::time::TimeManager;
use alloy_consensus::BlockHeader;
use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types_anvil::MineOptions;
use alloy_rpc_types_eth::Block;
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::{
    error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    ErrorObjectOwned,
};
use reth_storage_api::{BlockNumReader, HeaderProvider};
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

    #[method(name = "mine_detailed", aliases = ["evm_mine_detailed"])]
    async fn anvil_mine_detailed(&self, opts: Option<MineOptions>) -> RpcResult<Vec<Block>>;

    #[method(name = "dropTransaction")]
    async fn anvil_drop_transaction(&self, tx_hash: B256) -> RpcResult<Option<B256>>;

    #[method(name = "dropAllTransactions")]
    async fn anvil_drop_all_transactions(&self) -> RpcResult<()>;

    #[method(name = "removePoolTransactions")]
    async fn anvil_remove_pool_transactions(&self, address: Address) -> RpcResult<()>;

    #[method(name = "setLoggingEnabled")]
    async fn anvil_set_logging_enabled(&self, enabled: bool) -> RpcResult<()>;

    #[method(name = "getGenesisTime")]
    async fn anvil_get_genesis_time(&self) -> RpcResult<u64>;

    #[method(name = "setBlockTimestampInterval")]
    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> RpcResult<()>;

    #[method(name = "removeBlockTimestampInterval")]
    async fn anvil_remove_block_timestamp_interval(&self) -> RpcResult<bool>;
}

/// Implementation of the `anvil_*` RPC namespace.
#[derive(Debug, Clone)]
pub struct AnvilRpc<Pool, Provider, Blocks> {
    state: ImpersonationState,
    mining: MiningController,
    time: TimeManager,
    pool: Pool,
    provider: Provider,
    blocks: Blocks,
}

impl<Pool, Provider, Blocks> AnvilRpc<Pool, Provider, Blocks> {
    pub fn new(
        state: ImpersonationState,
        mining: MiningController,
        time: TimeManager,
        pool: Pool,
        provider: Provider,
        blocks: Blocks,
    ) -> Self {
        Self {
            state,
            mining,
            time,
            pool,
            provider,
            blocks,
        }
    }
}

fn internal_error(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, message.into(), None::<()>)
}

fn invalid_params(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, message.into(), None::<()>)
}

impl<Pool, Provider, Blocks> AnvilRpc<Pool, Provider, Blocks>
where
    Provider: BlockNumReader,
    Blocks: BlockSource<Block = Block>,
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

    fn best_block_number(&self) -> RpcResult<u64> {
        self.provider
            .best_block_number()
            .map_err(|error| internal_error(format!("failed to read latest block number: {error}")))
    }

    async fn mine_blocks(&self, blocks: u64) -> RpcResult<Vec<u64>> {
        if blocks == 0 {
            return Ok(Vec::new());
        }

        let start = self.best_block_number()?;

        for _ in 0..blocks {
            self.mining.trigger();
        }

        let end = start.saturating_add(blocks);
        // TODO(anvil): This assumes dev mining advances canon one block at a time with no
        // intervening head changes, which is fine for the current local-dev path but should be
        // revisited if manual mining ever needs stronger block attribution guarantees.
        self.wait_for_block_number(end).await?;

        Ok(((start + 1)..=end).collect())
    }
}

#[async_trait]
impl<Pool, Provider, Blocks> AnvilApiServer for AnvilRpc<Pool, Provider, Blocks>
where
    Pool: TransactionPool + Send + Sync + 'static,
    Provider: BlockNumReader + HeaderProvider + Send + Sync + 'static,
    Blocks: BlockSource<Block = Block>,
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
        self.mine_blocks(blocks).await?;
        Ok(())
    }

    async fn anvil_mine_detailed(&self, opts: Option<MineOptions>) -> RpcResult<Vec<Block>> {
        let (timestamp, blocks) = match opts.unwrap_or_default() {
            MineOptions::Options { timestamp, blocks } => (timestamp, blocks.unwrap_or(1)),
            MineOptions::Timestamp(timestamp) => (timestamp, 1),
        };

        if timestamp.is_some() {
            return Err(invalid_params(
                "anvil_mine_detailed timestamp is not supported yet",
            ));
        }

        let mined_blocks = self.mine_blocks(blocks).await?;
        let mut blocks = Vec::with_capacity(mined_blocks.len());

        for block_number in mined_blocks {
            let block = self
                .blocks
                .block_by_number_full(block_number)
                .await?
                .ok_or_else(|| internal_error(format!("missing mined block {block_number}")))?;
            blocks.push(block);
        }

        Ok(blocks)
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

    async fn anvil_get_genesis_time(&self) -> RpcResult<u64> {
        let header = self
            .provider
            .sealed_header(0)
            .map_err(|e| internal_error(format!("failed to read genesis header: {e}")))?
            .ok_or_else(|| internal_error("genesis block not found"))?;
        Ok(header.timestamp())
    }

    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> RpcResult<()> {
        self.time.set_block_timestamp_interval(seconds);
        Ok(())
    }

    async fn anvil_remove_block_timestamp_interval(&self) -> RpcResult<bool> {
        Ok(self.time.remove_block_timestamp_interval())
    }
}
