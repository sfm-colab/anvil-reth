use alloy_eips::BlockNumberOrTag;
use alloy_network::Ethereum;
use alloy_primitives::U256;
use alloy_rpc_types_eth::Block;
use jsonrpsee::core::{async_trait, RpcResult};
use reth_rpc_eth_api::{EthApiServer, FullEthApiServer};

/// Adapter for fetching mined blocks in the RPC response format without coupling the
/// mining-control flow to a concrete network API.
#[async_trait]
pub trait BlockSource: Clone + Send + Sync + 'static {
    type Block;

    async fn gas_price(&self) -> RpcResult<U256>;

    async fn block_by_number(&self, number: u64) -> RpcResult<Option<Self::Block>>;

    async fn block_by_number_full(&self, number: u64) -> RpcResult<Option<Self::Block>>;
}

#[async_trait]
impl<Eth> BlockSource for Eth
where
    Eth: FullEthApiServer<NetworkTypes = Ethereum> + 'static,
{
    type Block = Block;

    async fn gas_price(&self) -> RpcResult<U256> {
        EthApiServer::gas_price(self).await
    }

    async fn block_by_number(&self, number: u64) -> RpcResult<Option<Block>> {
        EthApiServer::block_by_number(self, BlockNumberOrTag::Number(number), false).await
    }

    async fn block_by_number_full(&self, number: u64) -> RpcResult<Option<Block>> {
        EthApiServer::block_by_number(self, BlockNumberOrTag::Number(number), true).await
    }
}
