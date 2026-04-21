use crate::{
    anvil_api::{AnvilApiServer, AnvilRpc},
    evm::AnvilExecutorBuilder,
    impersonation::{ImpersonatedSigner, ImpersonationState},
    mining::{run_automine_task, run_interval_mining_task, MiningController},
    pool::AnvilPoolBuilder,
    time::TimeManager,
};
use eyre::{OptionExt, Result};
use jsonrpsee::http_client::HttpClient;
use reth_db_mem::MemoryDatabase;
use reth_engine_local::MiningMode as LocalMiningMode;
use reth_ethereum::{
    chainspec::DEV,
    node::{
        builder::{components::NoopNetworkBuilder, NodeBuilder, NodeHandle},
        core::{
            args::{DatadirArgs, RpcServerArgs},
            dirs::{DataDirPath, MaybePlatformPath},
            node_config::NodeConfig,
        },
        node::EthereumAddOns,
        EthereumNode,
    },
    tasks::Runtime,
};
use std::{future::Future, sync::Arc};

fn test_node_config() -> NodeConfig<reth_ethereum::chainspec::ChainSpec> {
    let datadir = MaybePlatformPath::<DataDirPath>::from(tempfile::tempdir().unwrap().keep());
    NodeConfig::test()
        .with_chain(DEV.clone())
        .dev()
        .with_rpc(RpcServerArgs::default().with_unused_ports().with_http())
        .with_datadir_args(DatadirArgs {
            datadir,
            ..Default::default()
        })
}

pub(crate) async fn with_test_client<F, Fut>(test: F) -> Result<()>
where
    F: FnOnce(HttpClient) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let runtime = Runtime::test();
    let node_config = test_node_config();
    let impersonation = ImpersonationState::default();
    let mining = MiningController::default();
    let trigger_stream = mining.trigger_stream();

    let NodeHandle { node, .. } = NodeBuilder::new(node_config)
        .with_database(Arc::new(MemoryDatabase::new()))
        .with_launch_context(runtime.clone())
        .with_types::<EthereumNode>()
        .with_components(
            EthereumNode::components()
                .network(NoopNetworkBuilder::eth())
                .pool(AnvilPoolBuilder {
                    state: impersonation.clone(),
                })
                .executor(AnvilExecutorBuilder {
                    state: impersonation.clone(),
                }),
        )
        .with_add_ons(EthereumAddOns::default())
        .extend_rpc_modules({
            let impersonation = impersonation.clone();
            let mining = mining.clone();
            move |ctx| {
                ctx.registry
                    .eth_api()
                    .signers()
                    .write()
                    .push(Box::new(ImpersonatedSigner::new(impersonation.clone())));
                ctx.modules.merge_configured(
                    AnvilRpc::new(
                        impersonation,
                        mining,
                        TimeManager::default(),
                        ctx.pool().clone(),
                        ctx.provider().clone(),
                        ctx.registry.eth_api().clone(),
                    )
                    .into_rpc(),
                )?;
                Ok(())
            }
        })
        .launch_with_debug_capabilities()
        .with_mining_mode(LocalMiningMode::trigger(trigger_stream))
        .await?;

    node.task_executor.spawn_critical_task(
        "anvil automine control",
        run_automine_task(node.pool.clone(), mining.clone()),
    );
    node.task_executor.spawn_critical_task(
        "anvil interval mining control",
        run_interval_mining_task(mining.clone()),
    );

    let client = node
        .rpc_server_handles
        .rpc
        .http_client()
        .ok_or_eyre("http rpc client not available")?;

    test(client).await
}
