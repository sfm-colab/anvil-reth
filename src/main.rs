mod anvil_api;
mod impersonation;
mod pool;

use anvil_api::{AnvilApiServer, AnvilRpc};
use impersonation::ImpersonationState;
use pool::AnvilPoolBuilder;
use reth_ethereum::{
    node::{
        builder::{components::NoopNetworkBuilder, NodeBuilder, NodeHandle},
        core::{args::RpcServerArgs, node_config::NodeConfig},
        node::EthereumAddOns,
        EthereumNode,
    },
    tasks::Runtime,
};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let runtime = Runtime::test();
    let node_config = NodeConfig::test()
        .dev()
        .with_rpc(RpcServerArgs::default().with_http());

    let impersonation = ImpersonationState::default();

    let NodeHandle {
        node,
        node_exit_future,
    } = NodeBuilder::new(node_config)
        .testing_node(runtime)
        .with_types::<EthereumNode>()
        .with_components(
            EthereumNode::components()
                .network(NoopNetworkBuilder::eth())
                .pool(AnvilPoolBuilder {
                    state: impersonation.clone(),
                }),
        )
        .with_add_ons(EthereumAddOns::default())
        .extend_rpc_modules({
            let impersonation = impersonation.clone();
            move |ctx| {
                ctx.modules
                    .merge_configured(AnvilRpc::new(impersonation).into_rpc())?;
                Ok(())
            }
        })
        .launch_with_debug_capabilities()
        .await?;

    println!(
        "anvil-reth dev node started on {:?}",
        node.rpc_server_handles.rpc.http_local_addr()
    );

    node_exit_future.await?;

    Ok(())
}
