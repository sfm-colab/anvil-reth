use crate::impersonation::ImpersonationState;
use alloy_consensus::Header;
use alloy_eips::Decodable2718;
use alloy_primitives::Bytes;
use alloy_rpc_types_engine::ExecutionData;
use eyre::Result;
use reth_ethereum::{
    Block,
    chainspec::EthereumHardforks,
    evm::{primitives as reth_evm, EthEvmConfig},
    node::builder::{components::ExecutorBuilder, BuilderContext, FullNodeTypes, NodeTypes},
    primitives::{SealedBlock, SealedHeader, SignedTransaction},
    EthPrimitives, TransactionSigned,
};
use reth_evm::{
    ConfigureEngineEvm, ConfigureEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor,
};
use std::{
    error::Error,
    fmt::{self, Debug, Display, Formatter},
};

/// Wraps an inner EVM config and overrides sender recovery for impersonated
/// transactions during engine payload execution.
#[derive(Debug, Clone)]
pub struct AnvilEvmConfig<Evm> {
    inner: Evm,
    state: ImpersonationState,
}

impl<Evm> AnvilEvmConfig<Evm> {
    pub const fn new(inner: Evm, state: ImpersonationState) -> Self {
        Self { inner, state }
    }
}

impl<Evm> ConfigureEvm for AnvilEvmConfig<Evm>
where
    Evm: ConfigureEvm<Primitives = EthPrimitives>,
{
    type Primitives = <Evm as ConfigureEvm>::Primitives;
    type Error = <Evm as ConfigureEvm>::Error;
    type NextBlockEnvCtx = <Evm as ConfigureEvm>::NextBlockEnvCtx;
    type BlockExecutorFactory = <Evm as ConfigureEvm>::BlockExecutorFactory;
    type BlockAssembler = <Evm as ConfigureEvm>::BlockAssembler;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        self.inner.block_executor_factory()
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        self.inner.block_assembler()
    }

    fn evm_env(
        &self,
        header: &Header,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &Self::NextBlockEnvCtx,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.next_evm_env(parent, attributes)
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<Block>,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<ExecutionCtxFor<'_, Self>, Self::Error> {
        self.inner.context_for_next_block(parent, attributes)
    }
}

/// Simple error wrapper for tx decoding/recovery failures.
#[derive(Debug)]
pub struct TxConvertError(String);

impl Display for TxConvertError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for TxConvertError {}

impl<Evm> ConfigureEngineEvm<ExecutionData> for AnvilEvmConfig<Evm>
where
    Evm: ConfigureEvm<Primitives = EthPrimitives> + ConfigureEngineEvm<ExecutionData>,
{
    fn evm_env_for_payload(
        &self,
        payload: &ExecutionData,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env_for_payload(payload)
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a ExecutionData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &ExecutionData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        let txs = payload.payload.transactions().clone();
        let state = self.state.clone();

        let convert = move |raw: Bytes| {
            let tx = TransactionSigned::decode_2718_exact(raw.as_ref())
                .map_err(|e| TxConvertError(e.to_string()))?;
            let hash = tx.recalculate_hash();
            let signer = match state.tx_sender(&hash) {
                Some(sender) => sender,
                None => tx
                    .try_recover()
                    .map_err(|e| TxConvertError(e.to_string()))?,
            };
            Ok::<_, TxConvertError>(tx.with_signer(signer))
        };

        Ok((txs, convert))
    }
}

/// Executor builder that produces [`AnvilEvmConfig`].
#[derive(Debug, Clone)]
pub struct AnvilExecutorBuilder {
    pub state: ImpersonationState,
}

impl<Types, Node> ExecutorBuilder<Node> for AnvilExecutorBuilder
where
    Types: NodeTypes<
        ChainSpec: EthereumHardforks + Clone + Debug,
        Primitives = EthPrimitives,
    >,
    Node: FullNodeTypes<Types = Types>,
    EthEvmConfig<Types::ChainSpec>:
        ConfigureEvm<Primitives = EthPrimitives> + ConfigureEngineEvm<ExecutionData>,
{
    type EVM = AnvilEvmConfig<EthEvmConfig<Types::ChainSpec>>;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> Result<Self::EVM> {
        Ok(AnvilEvmConfig::new(
            EthEvmConfig::new(ctx.chain_spec()),
            self.state,
        ))
    }
}
