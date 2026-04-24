use crate::block_env::BlockEnvOverrides;
use crate::block_source::BlockSource;
use crate::impersonation::ImpersonationState;
use crate::mining::MiningController;
use crate::state::SharedAnvilState;
use crate::state_dump::SerializableState;
use crate::time::TimeManager;
use alloy_consensus::BlockHeader;
use alloy_network::{Ethereum, TransactionBuilder};
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rpc_types_anvil::{Metadata, MineOptions, NodeEnvironment, NodeForkConfig, NodeInfo};
use alloy_rpc_types_eth::{
    state::{AccountOverride, StateOverridesBuilder},
    Block, TransactionRequest,
};
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::{
    error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    ErrorObjectOwned,
};
use reth_ethereum::chainspec::{ChainSpec, EthChainSpec, EthereumHardforks};
use reth_rpc_eth_api::{EthApiServer, FullEthApiServer};
use reth_storage_api::{AccountReader, BlockNumReader, HeaderProvider, StateDumpProvider};
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

    #[method(name = "increaseTime", aliases = ["evm_increaseTime"])]
    async fn anvil_increase_time(&self, seconds: U256) -> RpcResult<i64>;

    #[method(name = "setTime", aliases = ["evm_setTime"])]
    async fn anvil_set_time(&self, timestamp: u64) -> RpcResult<u64>;

    #[method(
        name = "setNextBlockTimestamp",
        aliases = ["evm_setNextBlockTimestamp"]
    )]
    async fn anvil_set_next_block_timestamp(&self, seconds: u64) -> RpcResult<()>;

    #[method(name = "setBalance", aliases = ["hardhat_setBalance"])]
    async fn anvil_set_balance(&self, address: Address, balance: U256) -> RpcResult<()>;

    #[method(name = "addBalance", aliases = ["hardhat_addBalance"])]
    async fn anvil_add_balance(&self, address: Address, balance: U256) -> RpcResult<()>;

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

    #[method(name = "dumpState", aliases = ["hardhat_dumpState"])]
    async fn anvil_dump_state(&self, preserve_historical_states: Option<bool>) -> RpcResult<Bytes>;

    #[method(name = "loadState", aliases = ["hardhat_loadState"])]
    async fn anvil_load_state(&self, state: Bytes) -> RpcResult<bool>;

    #[method(name = "dealERC20", aliases = ["hardhat_dealERC20", "setERC20Balance"])]
    async fn anvil_deal_erc20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()>;

    #[method(name = "setERC20Allowance")]
    async fn anvil_set_erc20_allowance(
        &self,
        owner: Address,
        spender: Address,
        token_address: Address,
        amount: U256,
    ) -> RpcResult<()>;

    #[method(name = "setBlockGasLimit")]
    async fn anvil_set_block_gas_limit(&self, gas_limit: U256) -> RpcResult<bool>;

    #[method(name = "setCoinbase")]
    async fn anvil_set_coinbase(&self, address: Address) -> RpcResult<()>;

    #[method(name = "setNextBlockBaseFeePerGas")]
    async fn anvil_set_next_block_base_fee_per_gas(&self, base_fee: U256) -> RpcResult<()>;
}

/// `evm_*` RPC namespace.
///
/// Hosts methods that Foundry/Anvil expose under the `evm_` prefix rather than `anvil_`.
#[rpc(server, namespace = "evm")]
pub trait EvmApi {
    #[method(name = "setBlockGasLimit")]
    async fn evm_set_block_gas_limit(&self, gas_limit: U256) -> RpcResult<bool>;
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
    block_env: BlockEnvOverrides,
    node: AnvilNodeConfig,
}

impl AnvilContext {
    pub fn new(
        anvil_state: SharedAnvilState,
        block_env: BlockEnvOverrides,
        node: AnvilNodeConfig,
    ) -> Self {
        Self {
            anvil_state,
            block_env,
            node,
        }
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

    fn set_block_gas_limit(&self, gas_limit: U256) -> RpcResult<bool> {
        if gas_limit > U256::from(u64::MAX) {
            return Err(invalid_params("gas_limit exceeds u64::MAX"));
        }
        self.context.block_env.set_gas_limit(gas_limit.to::<u64>());
        Ok(true)
    }
}

fn internal_error(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, message.into(), None::<()>)
}

fn invalid_params(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, message.into(), None::<()>)
}

fn normalize_timestamp_input(timestamp: u64) -> u64 {
    if timestamp > 1_000_000_000_000 {
        timestamp / 1000
    } else {
        timestamp
    }
}

impl<Pool, Provider, Blocks> AnvilRpc<Pool, Provider, Blocks>
where
    Provider: AccountReader + BlockNumReader + StateDumpProvider,
    Blocks: BlockSource<Block = Block> + FullEthApiServer<NetworkTypes = Ethereum>,
{
    async fn wait_for_block_number(&self, expected: u64) -> RpcResult<()> {
        for _ in 0..200 {
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
        BlockSource::block_by_number(&self.blocks, number)
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

    fn account_balance(&self, address: Address) -> RpcResult<U256> {
        if let Some(balance) = self
            .context
            .anvil_state
            .read()
            .account(address)
            .and_then(|account| account.balance())
        {
            return Ok(balance);
        }

        Ok(self
            .provider
            .basic_account(&address)
            .map_err(|error| internal_error(format!("failed to read account: {error}")))?
            .unwrap_or_default()
            .balance)
    }

    fn account_nonce(&self, address: Address) -> RpcResult<u64> {
        let base_nonce = self
            .provider
            .basic_account(&address)
            .map_err(|error| internal_error(format!("failed to read account: {error}")))?
            .unwrap_or_default()
            .nonce;

        Ok(self
            .context
            .anvil_state
            .read()
            .account(address)
            .and_then(|account| account.nonce())
            .unwrap_or(base_nonce))
    }

    fn dump_state(&self, preserve_historical_states: Option<bool>) -> RpcResult<Bytes> {
        if preserve_historical_states.unwrap_or(false) {
            return Err(invalid_params(
                "preserving historical states is not supported yet",
            ));
        }

        let dump = self
            .provider
            .dump_state_collect()
            .map_err(|error| internal_error(format!("failed to dump state: {error}")))?;
        let mut state = SerializableState::from_dump(dump, Some(self.best_block_number()?));
        state.merge_anvil_state(&self.context.anvil_state.read());
        state
            .encode_gzipped()
            .map_err(|error| internal_error(format!("failed to encode state dump: {error}")))
    }

    fn load_state(&self, buf: Bytes) -> RpcResult<bool> {
        let state = SerializableState::decode(&buf)
            .map_err(|error| invalid_params(format!("failed to decode state dump: {error}")))?;

        for (address, account) in state.accounts {
            let nonce = self.account_nonce(address)?.max(account.nonce);
            let mut anvil_state = self.context.anvil_state.write();
            anvil_state.set_balance(address, account.balance);
            anvil_state.set_nonce(address, nonce);
            anvil_state.set_code(address, account.code);

            for (slot, value) in account.storage {
                anvil_state.set_storage_at(address, slot, value.into());
            }
        }

        Ok(true)
    }

    async fn find_erc20_storage_slot(
        &self,
        token_address: Address,
        calldata: Bytes,
        expected_value: U256,
    ) -> RpcResult<B256> {
        let tx = TransactionRequest::default()
            .with_to(token_address)
            .with_input(calldata.clone());

        let access_list = EthApiServer::create_access_list(&self.blocks, tx.clone(), None, None)
            .await?
            .access_list;

        for item in access_list.0 {
            if item.address != token_address {
                continue;
            }

            for slot in item.storage_keys {
                let state_override = StateOverridesBuilder::default()
                    .append(
                        token_address,
                        AccountOverride::default().with_state_diff(std::iter::once((
                            slot,
                            B256::from(expected_value.to_be_bytes()),
                        ))),
                    )
                    .build();

                let Ok(result) =
                    EthApiServer::call(&self.blocks, tx.clone(), None, Some(state_override), None)
                        .await
                else {
                    continue;
                };

                if U256::from_be_slice(result.as_ref()) == expected_value {
                    return Ok(slot);
                }
            }
        }

        Err(internal_error("Unable to find storage slot"))
    }
}

#[async_trait]
impl<Pool, Provider, Blocks> AnvilApiServer for AnvilRpc<Pool, Provider, Blocks>
where
    Pool: TransactionPool + Send + Sync + 'static,
    Provider:
        AccountReader + BlockNumReader + HeaderProvider + StateDumpProvider + Send + Sync + 'static,
    Blocks: BlockSource<Block = Block> + FullEthApiServer<NetworkTypes = Ethereum>,
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
        let blocks = num_blocks.unwrap_or(U256::from(1)).to::<u64>();

        if let Some(interval) = interval.filter(|interval| !interval.is_zero()) {
            let previous_interval = self.time.interval();
            self.time.set_block_timestamp_interval(interval.to::<u64>());
            let result = self.mine_blocks(blocks).await;
            match previous_interval {
                Some(previous_interval) => {
                    self.time.set_block_timestamp_interval(previous_interval)
                }
                None => {
                    self.time.remove_block_timestamp_interval();
                }
            }
            result?;
            return Ok(());
        }

        self.mine_blocks(blocks).await?;
        Ok(())
    }

    async fn anvil_mine_detailed(&self, opts: Option<MineOptions>) -> RpcResult<Vec<Block>> {
        let (timestamp, blocks) = match opts.unwrap_or_default() {
            MineOptions::Options { timestamp, blocks } => (timestamp, blocks.unwrap_or(1)),
            MineOptions::Timestamp(timestamp) => (timestamp, 1),
        };

        if let Some(timestamp) = timestamp {
            self.time
                .set_next_block_timestamp(timestamp)
                .map_err(invalid_params)?;
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
        let gas_price = BlockSource::gas_price(&self.blocks).await?;

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

    async fn anvil_increase_time(&self, seconds: U256) -> RpcResult<i64> {
        let offset = self.time.increase_time(seconds.to::<u64>());
        Ok(offset.min(i64::MAX as i128) as i64)
    }

    async fn anvil_set_time(&self, timestamp: u64) -> RpcResult<u64> {
        let timestamp = normalize_timestamp_input(timestamp);
        let now = self.time.current_call_timestamp();
        self.time.set_time(timestamp);
        Ok(timestamp.saturating_sub(now))
    }

    async fn anvil_set_next_block_timestamp(&self, seconds: u64) -> RpcResult<()> {
        self.time
            .set_next_block_timestamp(seconds)
            .map_err(invalid_params)?;
        Ok(())
    }

    async fn anvil_set_balance(&self, address: Address, balance: U256) -> RpcResult<()> {
        self.context
            .anvil_state
            .write()
            .set_balance(address, balance);
        Ok(())
    }

    async fn anvil_add_balance(&self, address: Address, balance: U256) -> RpcResult<()> {
        let current = self.account_balance(address)?;
        self.context
            .anvil_state
            .write()
            .set_balance(address, current.saturating_add(balance));
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

    async fn anvil_dump_state(&self, preserve_historical_states: Option<bool>) -> RpcResult<Bytes> {
        self.dump_state(preserve_historical_states)
    }

    async fn anvil_load_state(&self, state: Bytes) -> RpcResult<bool> {
        self.load_state(state)
    }

    async fn anvil_deal_erc20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()> {
        const BALANCE_OF_SELECTOR: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];

        let mut calldata = Vec::with_capacity(4 + 32);
        calldata.extend_from_slice(&BALANCE_OF_SELECTOR);
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(address.as_slice());

        let slot = self
            .find_erc20_storage_slot(token_address, Bytes::from(calldata), balance)
            .await?;

        self.context.anvil_state.write().set_storage_at(
            token_address,
            slot,
            B256::from(balance.to_be_bytes()).into(),
        );
        Ok(())
    }

    async fn anvil_set_erc20_allowance(
        &self,
        owner: Address,
        spender: Address,
        token_address: Address,
        amount: U256,
    ) -> RpcResult<()> {
        const ALLOWANCE_SELECTOR: [u8; 4] = [0xdd, 0x62, 0xed, 0x3e];

        let mut calldata = Vec::with_capacity(4 + 32 + 32);
        calldata.extend_from_slice(&ALLOWANCE_SELECTOR);
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(owner.as_slice());
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(spender.as_slice());

        let slot = self
            .find_erc20_storage_slot(token_address, Bytes::from(calldata), amount)
            .await?;

        self.context.anvil_state.write().set_storage_at(
            token_address,
            slot,
            B256::from(amount.to_be_bytes()).into(),
        );
        Ok(())
    }

    async fn anvil_set_block_gas_limit(&self, gas_limit: U256) -> RpcResult<bool> {
        self.set_block_gas_limit(gas_limit)
    }

    async fn anvil_set_coinbase(&self, address: Address) -> RpcResult<()> {
        self.context.block_env.set_coinbase(address);
        Ok(())
    }

    async fn anvil_set_next_block_base_fee_per_gas(&self, base_fee: U256) -> RpcResult<()> {
        if base_fee > U256::from(u64::MAX) {
            return Err(invalid_params("base_fee exceeds u64::MAX"));
        }
        self.context
            .block_env
            .set_next_base_fee(base_fee.to::<u64>());
        Ok(())
    }
}

#[async_trait]
impl<Pool, Provider, Blocks> EvmApiServer for AnvilRpc<Pool, Provider, Blocks>
where
    Pool: TransactionPool + Send + Sync + 'static,
    Provider: AccountReader + BlockNumReader + HeaderProvider + Send + Sync + 'static,
    Blocks: BlockSource<Block = Block> + FullEthApiServer<NetworkTypes = Ethereum>,
{
    async fn evm_set_block_gas_limit(&self, gas_limit: U256) -> RpcResult<bool> {
        self.set_block_gas_limit(gas_limit)
    }
}
