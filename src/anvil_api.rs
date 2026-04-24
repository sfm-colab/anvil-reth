use crate::block_source::BlockSource;
use crate::impersonation::ImpersonationState;
use crate::mining::MiningController;
use crate::state::SharedAnvilState;
use crate::time::TimeManager;
use alloy_consensus::BlockHeader;
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rpc_types_anvil::{Metadata, MineOptions, NodeEnvironment, NodeForkConfig, NodeInfo};
use alloy_rpc_types_eth::Block;
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::{
    error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    ErrorObjectOwned,
};
use reth_ethereum::chainspec::{ChainSpec, EthChainSpec, EthereumHardforks};
use reth_storage_api::{BlockNumReader, HeaderProvider};
use reth_transaction_pool::TransactionPool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
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

    #[method(name = "nodeInfo")]
    async fn anvil_node_info(&self) -> RpcResult<NodeInfo>;

    #[method(name = "metadata", aliases = ["hardhat_metadata"])]
    async fn anvil_metadata(&self) -> RpcResult<Metadata>;

    #[method(name = "setBlockTimestampInterval")]
    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> RpcResult<()>;

    #[method(name = "removeBlockTimestampInterval")]
    async fn anvil_remove_block_timestamp_interval(&self) -> RpcResult<bool>;

    #[method(name = "setBalance", aliases = ["hardhat_setBalance"])]
    async fn anvil_set_balance(&self, address: Address, balance: U256) -> RpcResult<()>;

    #[method(name = "setNonce", aliases = ["hardhat_setNonce"])]
    async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> RpcResult<()>;

    #[method(name = "setCode", aliases = ["hardhat_setCode"])]
    async fn anvil_set_code(&self, address: Address, code: Bytes) -> RpcResult<()>;

    #[method(name = "setStorageAt", aliases = ["hardhat_setStorageAt"])]
    async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        value: B256,
    ) -> RpcResult<bool>;
}

/// Implementation of the `anvil_*` RPC namespace.
#[derive(Debug, Clone)]
pub struct AnvilRpc<Pool, Provider, Blocks> {
    state: ImpersonationState,
    mining: MiningController,
    time: TimeManager,
    context: AnvilContext,
    pool: Pool,
    provider: Provider,
    blocks: Blocks,
}

#[derive(Debug, Clone)]
pub struct AnvilContext {
    anvil_state: SharedAnvilState,
    node: AnvilNodeConfig,
}

impl AnvilContext {
    pub fn new(anvil_state: SharedAnvilState, node: AnvilNodeConfig) -> Self {
        Self { anvil_state, node }
    }
}

#[derive(Debug, Clone)]
pub struct AnvilNodeConfig {
    chain_spec: Arc<ChainSpec>,
    instance_id: B256,
}

impl AnvilNodeConfig {
    pub fn new(chain_spec: Arc<ChainSpec>, instance_id: B256) -> Self {
        Self {
            chain_spec,
            instance_id,
        }
    }

    fn hardfork_name(&self, timestamp: u64) -> String {
        let chain_spec = &self.chain_spec;

        if chain_spec.is_osaka_active_at_timestamp(timestamp) {
            "osaka"
        } else if chain_spec.is_prague_active_at_timestamp(timestamp) {
            "prague"
        } else if chain_spec.is_cancun_active_at_timestamp(timestamp) {
            "cancun"
        } else if chain_spec.is_shanghai_active_at_timestamp(timestamp) {
            "shanghai"
        } else {
            "london"
        }
        .to_string()
    }
}

impl<Pool, Provider, Blocks> AnvilRpc<Pool, Provider, Blocks> {
    pub fn new(
        state: ImpersonationState,
        mining: MiningController,
        time: TimeManager,
        context: AnvilContext,
        pool: Pool,
        provider: Provider,
        blocks: Blocks,
    ) -> Self {
        Self {
            state,
            mining,
            time,
            context,
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

    async fn latest_block(&self) -> RpcResult<Block> {
        let number = self.best_block_number()?;
        self.blocks
            .block_by_number(number)
            .await?
            .ok_or_else(|| internal_error(format!("missing block header {number}")))
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

    async fn anvil_node_info(&self) -> RpcResult<NodeInfo> {
        let latest = self.latest_block().await?;
        let gas_price = self.blocks.gas_price().await?;

        Ok(NodeInfo {
            current_block_number: latest.header.number,
            current_block_timestamp: latest.header.timestamp,
            current_block_hash: latest.header.hash,
            hard_fork: self.context.node.hardfork_name(latest.header.timestamp),
            transaction_order: "fees".to_string(),
            environment: NodeEnvironment {
                base_fee: latest.header.base_fee_per_gas.unwrap_or_default() as u128,
                chain_id: self.context.node.chain_spec.chain().id(),
                gas_limit: latest.header.gas_limit,
                gas_price: gas_price.to::<u128>(),
            },
            fork_config: NodeForkConfig::default(),
            network: None,
        })
    }

    async fn anvil_metadata(&self) -> RpcResult<Metadata> {
        let latest = self.latest_block().await?;

        Ok(Metadata {
            client_version: format!("{}/v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
            client_semver: Some(env!("CARGO_PKG_VERSION").to_string()),
            client_commit_sha: option_env!("VERGEN_GIT_SHA").map(ToString::to_string),
            chain_id: self.context.node.chain_spec.chain().id(),
            instance_id: self.context.node.instance_id,
            latest_block_number: latest.header.number,
            latest_block_hash: latest.header.hash,
            forked_network: None,
            snapshots: BTreeMap::new(),
        })
    }

    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> RpcResult<()> {
        self.time.set_block_timestamp_interval(seconds);
        Ok(())
    }

    async fn anvil_remove_block_timestamp_interval(&self) -> RpcResult<bool> {
        Ok(self.time.remove_block_timestamp_interval())
    }

    async fn anvil_set_balance(&self, address: Address, balance: U256) -> RpcResult<()> {
        self.context
            .anvil_state
            .write()
            .set_balance(address, balance);
        Ok(())
    }

    async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> RpcResult<()> {
        if nonce > U256::from(u64::MAX) {
            return Err(invalid_params("nonce exceeds u64::MAX"));
        }

        self.context
            .anvil_state
            .write()
            .set_nonce(address, nonce.to());
        Ok(())
    }

    async fn anvil_set_code(&self, address: Address, code: Bytes) -> RpcResult<()> {
        self.context.anvil_state.write().set_code(address, code);
        Ok(())
    }

    async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        value: B256,
    ) -> RpcResult<bool> {
        self.context.anvil_state.write().set_storage_at(
            address,
            slot.to_be_bytes().into(),
            value.into(),
        );
        Ok(true)
    }
}
