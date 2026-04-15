use crate::impersonation::ImpersonationState;
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rpc_types_anvil::{Forking, Metadata, MineOptions, NodeInfo};
use alloy_rpc_types_eth::Block;
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::proc_macros::rpc;

/// anvil_* RPC namespace.
///
/// Mirrors the trait defined in reth-rpc-api (which doesn't publicly export
/// `AnvilApiServer` in its `servers` module).
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

    #[method(name = "mine")]
    async fn anvil_mine(&self, blocks: Option<U256>, interval: Option<U256>) -> RpcResult<()>;

    #[method(name = "setAutomine")]
    async fn anvil_set_automine(&self, enabled: bool) -> RpcResult<()>;

    #[method(name = "setIntervalMining")]
    async fn anvil_set_interval_mining(&self, interval: u64) -> RpcResult<()>;

    #[method(name = "anvil_dropTransaction")]
    async fn anvil_drop_transaction(&self, tx_hash: B256) -> RpcResult<Option<B256>>;

    #[method(name = "reset")]
    async fn anvil_reset(&self, fork: Option<Forking>) -> RpcResult<()>;

    #[method(name = "setRpcUrl")]
    async fn anvil_set_rpc_url(&self, url: String) -> RpcResult<()>;

    #[method(name = "setBalance")]
    async fn anvil_set_balance(&self, address: Address, balance: U256) -> RpcResult<()>;

    #[method(name = "setCode")]
    async fn anvil_set_code(&self, address: Address, code: Bytes) -> RpcResult<()>;

    #[method(name = "setNonce")]
    async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> RpcResult<()>;

    #[method(name = "setStorageAt")]
    async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        value: B256,
    ) -> RpcResult<bool>;

    #[method(name = "setCoinbase")]
    async fn anvil_set_coinbase(&self, address: Address) -> RpcResult<()>;

    #[method(name = "setChainId")]
    async fn anvil_set_chain_id(&self, chain_id: u64) -> RpcResult<()>;

    #[method(name = "setLoggingEnabled")]
    async fn anvil_set_logging_enabled(&self, enabled: bool) -> RpcResult<()>;

    #[method(name = "setMinGasPrice")]
    async fn anvil_set_min_gas_price(&self, gas_price: U256) -> RpcResult<()>;

    #[method(name = "setNextBlockBaseFeePerGas")]
    async fn anvil_set_next_block_base_fee_per_gas(&self, base_fee: U256) -> RpcResult<()>;

    #[method(name = "setTime")]
    async fn anvil_set_time(&self, timestamp: u64) -> RpcResult<u64>;

    #[method(name = "dumpState")]
    async fn anvil_dump_state(&self) -> RpcResult<Bytes>;

    #[method(name = "loadState")]
    async fn anvil_load_state(&self, state: Bytes) -> RpcResult<bool>;

    #[method(name = "nodeInfo")]
    async fn anvil_node_info(&self) -> RpcResult<NodeInfo>;

    #[method(name = "metadata")]
    async fn anvil_metadata(&self) -> RpcResult<Metadata>;

    #[method(name = "snapshot")]
    async fn anvil_snapshot(&self) -> RpcResult<U256>;

    #[method(name = "revert")]
    async fn anvil_revert(&self, id: U256) -> RpcResult<bool>;

    #[method(name = "increaseTime")]
    async fn anvil_increase_time(&self, seconds: U256) -> RpcResult<i64>;

    #[method(name = "setNextBlockTimestamp")]
    async fn anvil_set_next_block_timestamp(&self, seconds: u64) -> RpcResult<()>;

    #[method(name = "setBlockGasLimit")]
    async fn anvil_set_block_gas_limit(&self, gas_limit: U256) -> RpcResult<bool>;

    #[method(name = "setBlockTimestampInterval")]
    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> RpcResult<()>;

    #[method(name = "removeBlockTimestampInterval")]
    async fn anvil_remove_block_timestamp_interval(&self) -> RpcResult<bool>;

    #[method(name = "mine_detailed")]
    async fn anvil_mine_detailed(&self, opts: Option<MineOptions>) -> RpcResult<Vec<Block>>;

    #[method(name = "enableTraces")]
    async fn anvil_enable_traces(&self) -> RpcResult<()>;

    #[method(name = "removePoolTransactions")]
    async fn anvil_remove_pool_transactions(&self, address: Address) -> RpcResult<()>;
}

/// Implementation of the `anvil_*` RPC namespace.
#[derive(Debug, Clone)]
pub struct AnvilRpc {
    state: ImpersonationState,
}

impl AnvilRpc {
    pub fn new(state: ImpersonationState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl AnvilApiServer for AnvilRpc {
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

    async fn anvil_mine(&self, _blocks: Option<U256>, _interval: Option<U256>) -> RpcResult<()> {
        Err(not_implemented("anvil_mine"))
    }

    async fn anvil_set_automine(&self, _enabled: bool) -> RpcResult<()> {
        Err(not_implemented("anvil_setAutomine"))
    }

    async fn anvil_set_interval_mining(&self, _interval: u64) -> RpcResult<()> {
        Err(not_implemented("anvil_setIntervalMining"))
    }

    async fn anvil_drop_transaction(&self, _tx_hash: B256) -> RpcResult<Option<B256>> {
        Err(not_implemented("anvil_dropTransaction"))
    }

    async fn anvil_reset(&self, _fork: Option<Forking>) -> RpcResult<()> {
        Err(not_implemented("anvil_reset"))
    }

    async fn anvil_set_rpc_url(&self, _url: String) -> RpcResult<()> {
        Err(not_implemented("anvil_setRpcUrl"))
    }

    async fn anvil_set_balance(&self, _address: Address, _balance: U256) -> RpcResult<()> {
        Err(not_implemented("anvil_setBalance"))
    }

    async fn anvil_set_code(&self, _address: Address, _code: Bytes) -> RpcResult<()> {
        Err(not_implemented("anvil_setCode"))
    }

    async fn anvil_set_nonce(&self, _address: Address, _nonce: U256) -> RpcResult<()> {
        Err(not_implemented("anvil_setNonce"))
    }

    async fn anvil_set_storage_at(
        &self,
        _address: Address,
        _slot: U256,
        _value: B256,
    ) -> RpcResult<bool> {
        Err(not_implemented("anvil_setStorageAt"))
    }

    async fn anvil_set_coinbase(&self, _address: Address) -> RpcResult<()> {
        Err(not_implemented("anvil_setCoinbase"))
    }

    async fn anvil_set_chain_id(&self, _chain_id: u64) -> RpcResult<()> {
        Err(not_implemented("anvil_setChainId"))
    }

    async fn anvil_set_logging_enabled(&self, _enabled: bool) -> RpcResult<()> {
        Ok(())
    }

    async fn anvil_set_min_gas_price(&self, _gas_price: U256) -> RpcResult<()> {
        Err(not_implemented("anvil_setMinGasPrice"))
    }

    async fn anvil_set_next_block_base_fee_per_gas(&self, _base_fee: U256) -> RpcResult<()> {
        Err(not_implemented("anvil_setNextBlockBaseFeePerGas"))
    }

    async fn anvil_set_time(&self, _timestamp: u64) -> RpcResult<u64> {
        Err(not_implemented("anvil_setTime"))
    }

    async fn anvil_dump_state(&self) -> RpcResult<Bytes> {
        Err(not_implemented("anvil_dumpState"))
    }

    async fn anvil_load_state(&self, _state: Bytes) -> RpcResult<bool> {
        Err(not_implemented("anvil_loadState"))
    }

    async fn anvil_node_info(&self) -> RpcResult<NodeInfo> {
        Err(not_implemented("anvil_nodeInfo"))
    }

    async fn anvil_metadata(&self) -> RpcResult<Metadata> {
        Err(not_implemented("anvil_metadata"))
    }

    async fn anvil_snapshot(&self) -> RpcResult<U256> {
        Err(not_implemented("anvil_snapshot"))
    }

    async fn anvil_revert(&self, _id: U256) -> RpcResult<bool> {
        Err(not_implemented("anvil_revert"))
    }

    async fn anvil_increase_time(&self, _seconds: U256) -> RpcResult<i64> {
        Err(not_implemented("anvil_increaseTime"))
    }

    async fn anvil_set_next_block_timestamp(&self, _seconds: u64) -> RpcResult<()> {
        Err(not_implemented("anvil_setNextBlockTimestamp"))
    }

    async fn anvil_set_block_gas_limit(&self, _gas_limit: U256) -> RpcResult<bool> {
        Err(not_implemented("anvil_setBlockGasLimit"))
    }

    async fn anvil_set_block_timestamp_interval(&self, _seconds: u64) -> RpcResult<()> {
        Err(not_implemented("anvil_setBlockTimestampInterval"))
    }

    async fn anvil_remove_block_timestamp_interval(&self) -> RpcResult<bool> {
        Err(not_implemented("anvil_removeBlockTimestampInterval"))
    }

    async fn anvil_mine_detailed(&self, _opts: Option<MineOptions>) -> RpcResult<Vec<Block>> {
        Err(not_implemented("anvil_mine_detailed"))
    }

    async fn anvil_enable_traces(&self) -> RpcResult<()> {
        Err(not_implemented("anvil_enableTraces"))
    }

    async fn anvil_remove_pool_transactions(&self, _address: Address) -> RpcResult<()> {
        Err(not_implemented("anvil_removePoolTransactions"))
    }
}

fn not_implemented(method: &str) -> jsonrpsee::types::ErrorObject<'static> {
    jsonrpsee::types::ErrorObject::owned(
        jsonrpsee::types::error::INTERNAL_ERROR_CODE,
        format!("{method} is not yet implemented"),
        None::<()>,
    )
}
