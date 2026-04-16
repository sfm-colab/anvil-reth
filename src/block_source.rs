use alloy_eips::BlockNumberOrTag;
use alloy_network::Ethereum;
use alloy_rpc_types_eth::Block;
use jsonrpsee::core::{async_trait, RpcResult};
use reth_rpc_eth_api::FullEthApiServer;

/// Adapter for fetching mined blocks in the RPC response format without coupling the
/// mining-control flow to a concrete network API.
#[async_trait]
pub trait BlockSource: Clone + Send + Sync + 'static {
    type Block;

    async fn block_by_number_full(&self, number: u64) -> RpcResult<Option<Self::Block>>;
}

#[async_trait]
impl<Eth> BlockSource for Eth
where
    Eth: FullEthApiServer<NetworkTypes = Ethereum> + Send + Sync + 'static,
{
    type Block = Block;

    async fn block_by_number_full(&self, number: u64) -> RpcResult<Option<Block>> {
        self.block_by_number(BlockNumberOrTag::Number(number), true)
            .await
    }
}
