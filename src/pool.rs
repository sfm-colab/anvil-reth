use alloy_consensus::Transaction;

use crate::impersonation::ImpersonationState;
use reth_ethereum::{
    chainspec::EthereumHardforks,
    node::{
        builder::{
            components::{create_blob_store_with_cache, PoolBuilder, TxPoolBuilder},
            BuilderContext, FullNodeTypes, NodeTypes,
        },
        api::PrimitivesTy,
    },
    pool::{
        blobstore::DiskFileBlobStore, validate::ValidTransaction, CoinbaseTipOrdering,
        EthPooledTransaction, EthTransactionValidator, Pool, PoolTransaction, TransactionOrigin,
        TransactionValidationOutcome, TransactionValidationTaskExecutor, TransactionValidator,
    },
    primitives::SealedBlock,
    TransactionSigned,
};
use std::fmt;

/// Wraps the standard Ethereum validator and short-circuits validation for
/// impersonated accounts.
pub struct AnvilValidator<V> {
    inner: V,
    state: ImpersonationState,
}

impl<V: fmt::Debug> fmt::Debug for AnvilValidator<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilValidator")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<V> TransactionValidator for AnvilValidator<V>
where
    V: TransactionValidator,
    V::Transaction: PoolTransaction,
{
    type Transaction = V::Transaction;
    type Block = V::Block;

    async fn validate_transaction(
        &self,
        origin: TransactionOrigin,
        transaction: Self::Transaction,
    ) -> TransactionValidationOutcome<Self::Transaction> {
        let sender = transaction.sender();

        if self.state.is_impersonated(&sender) {
            return TransactionValidationOutcome::Valid {
                balance: alloy_primitives::U256::MAX,
                state_nonce: transaction.nonce(),
                bytecode_hash: None,
                transaction: ValidTransaction::Valid(transaction),
                propagate: false,
                authorities: None,
            };
        }

        self.inner.validate_transaction(origin, transaction).await
    }

    fn on_new_head_block(&self, new_tip_block: &SealedBlock<Self::Block>) {
        self.inner.on_new_head_block(new_tip_block)
    }
}

/// Pool builder that wraps `EthereumPoolBuilder` and decorates the validator
/// with impersonation support.
#[derive(Debug, Clone)]
pub struct AnvilPoolBuilder {
    pub state: ImpersonationState,
}

/// The concrete pool type produced by `AnvilPoolBuilder`.
pub type AnvilTransactionPool<Provider, Evm> = Pool<
    TransactionValidationTaskExecutor<
        AnvilValidator<EthTransactionValidator<Provider, EthPooledTransaction, Evm>>,
    >,
    CoinbaseTipOrdering<EthPooledTransaction>,
    DiskFileBlobStore,
>;

impl<Types, Node, Evm> PoolBuilder<Node, Evm> for AnvilPoolBuilder
where
    Types: NodeTypes<
        ChainSpec: EthereumHardforks,
        Primitives: reth_ethereum::node::api::NodePrimitives<SignedTx = TransactionSigned>,
    >,
    Node: FullNodeTypes<Types = Types>,
    Evm: reth_ethereum::node::api::ConfigureEvm<Primitives = PrimitivesTy<Types>> + Clone + 'static,
{
    type Pool = AnvilTransactionPool<Node::Provider, Evm>;

    async fn build_pool(
        self,
        ctx: &BuilderContext<Node>,
        evm_config: Evm,
    ) -> eyre::Result<Self::Pool> {
        let pool_config = ctx.pool_config();
        let blob_store = create_blob_store_with_cache(ctx, None)?;

        let executor =
            TransactionValidationTaskExecutor::eth_builder(ctx.provider().clone(), evm_config)
                .kzg_settings(ctx.kzg_settings()?)
                .with_max_tx_input_bytes(ctx.config().txpool.max_tx_input_bytes)
                .with_local_transactions_config(pool_config.local_transactions_config.clone())
                .set_tx_fee_cap(ctx.config().rpc.rpc_tx_fee_cap)
                .with_max_tx_gas_limit(ctx.config().txpool.max_tx_gas_limit)
                .with_minimum_priority_fee(ctx.config().txpool.minimum_priority_fee)
                .with_additional_tasks(ctx.config().txpool.additional_validation_tasks)
                .build_with_tasks(ctx.task_executor().clone(), blob_store.clone());

        let mut state = Some(self.state);
        let anvil_executor =
            executor.map(move |inner| AnvilValidator { inner, state: state.take().unwrap() });

        let transaction_pool = TxPoolBuilder::new(ctx)
            .with_validator(anvil_executor)
            .build_and_spawn_maintenance_task(blob_store, pool_config)?;

        Ok(transaction_pool)
    }
}
