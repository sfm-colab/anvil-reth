use crate::{state::SharedAnvilState, state_provider::AnvilStateProvider};
use alloy_network::Ethereum;
use reth_ethereum::{
    chainspec::{EthereumHardforks, Hardforks},
    node::{
        builder::{
            rpc::{
                BasicEngineApiBuilder, BasicEngineValidatorBuilder,
                EthApiBuilder as NodeEthApiBuilder, EthApiCtx, Identity, RpcAddOns,
            },
            ConfigureEvm, FullNodeComponents, HeaderTy, NodeTypes, PrimitivesTy, TxTy,
        },
        node::{EthereumAddOns, EthereumEngineValidatorBuilder},
    },
};
use reth_rpc::{
    eth::core::{EthApiFor, EthRpcConverterFor},
};
use reth_rpc_eth_api::{
    helpers::pending_block::BuildPendingEnv, RpcConvert, RpcTypes, SignableTxRequest,
};
use reth_rpc_eth_types::{error::FromEvmError, EthApiError};

/// Custom ETH API builder that installs the local Anvil state interceptor once, so stock Reth
/// `eth_*` handlers resolve through `AnvilStateProvider` without per-method RPC overrides.
#[derive(Debug, Clone)]
pub struct AnvilEthApiBuilder {
    state: SharedAnvilState,
}

impl AnvilEthApiBuilder {
    pub fn new(state: SharedAnvilState) -> Self {
        Self { state }
    }
}

impl Default for AnvilEthApiBuilder {
    fn default() -> Self {
        Self::new(crate::state::AnvilState::shared())
    }
}

impl<N> NodeEthApiBuilder<N> for AnvilEthApiBuilder
where
    N: FullNodeComponents<
        Types: NodeTypes<ChainSpec: Hardforks + EthereumHardforks>,
        Evm: ConfigureEvm<NextBlockEnvCtx: BuildPendingEnv<HeaderTy<N::Types>>>,
    >,
    Ethereum: RpcTypes<TransactionRequest: SignableTxRequest<TxTy<N::Types>>>,
    EthRpcConverterFor<N>: RpcConvert<
        Primitives = PrimitivesTy<N::Types>,
        Error = EthApiError,
        Network = Ethereum,
        Evm = N::Evm,
    >,
    EthApiError: FromEvmError<N::Evm>,
{
    type EthApi = EthApiFor<N>;

    async fn build_eth_api(self, ctx: EthApiCtx<'_, N>) -> eyre::Result<Self::EthApi> {
        let state = self.state.clone();

        Ok(ctx
            .eth_api_builder()
            .map_converter(|rpc| rpc.with_network())
            .interceptor(move |inner| Box::new(AnvilStateProvider::new(state.clone(), inner)))
            .build())
    }
}

pub fn anvil_add_ons<N>(
    state: SharedAnvilState,
) -> EthereumAddOns<N, AnvilEthApiBuilder, EthereumEngineValidatorBuilder>
where
    N: FullNodeComponents<
        Types: NodeTypes<ChainSpec: Hardforks + EthereumHardforks>,
        Evm: ConfigureEvm<NextBlockEnvCtx: BuildPendingEnv<HeaderTy<N::Types>>>,
    >,
    Ethereum: RpcTypes<TransactionRequest: SignableTxRequest<TxTy<N::Types>>>,
    EthRpcConverterFor<N>: RpcConvert<
        Primitives = PrimitivesTy<N::Types>,
        Error = EthApiError,
        Network = Ethereum,
        Evm = N::Evm,
    >,
    EthApiError: FromEvmError<N::Evm>,
{
    EthereumAddOns::new(RpcAddOns::new(
        AnvilEthApiBuilder::new(state),
        EthereumEngineValidatorBuilder::default(),
        BasicEngineApiBuilder::<EthereumEngineValidatorBuilder>::default(),
        BasicEngineValidatorBuilder::<EthereumEngineValidatorBuilder>::default(),
        Default::default(),
        Identity::new(),
    ))
}
