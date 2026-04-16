mod anvil_api;
mod evm;
mod impersonation;
mod pool;

use anvil_api::{AnvilApiServer, AnvilRpc};
use evm::AnvilExecutorBuilder;
use impersonation::{ImpersonatedSigner, ImpersonationState};
use pool::AnvilPoolBuilder;
#[cfg(test)]
use alloy_network::TransactionBuilder;
#[cfg(test)]
use alloy_primitives::{Address, B256, U256};
#[cfg(test)]
use alloy_rpc_types_eth::TransactionRequest;
use eyre::Result;
#[cfg(test)]
use eyre::{bail, OptionExt};
#[cfg(test)]
use jsonrpsee::{core::{client::ClientT, ClientError}, http_client::HttpClient, rpc_params};
use reth_ethereum::{
    chainspec::DEV,
    node::{
        builder::{components::NoopNetworkBuilder, NodeBuilder, NodeHandle},
        core::{args::RpcServerArgs, node_config::NodeConfig},
        node::EthereumAddOns,
        EthereumNode,
    },
    tasks::Runtime,
};
#[cfg(test)]
use serde_json::Value;
#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    let runtime = Runtime::test();
    let node_config = NodeConfig::test()
        .with_chain(DEV.clone())
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
                })
                .executor(AnvilExecutorBuilder {
                    state: impersonation.clone(),
                }),
        )
        .with_add_ons(EthereumAddOns::default())
        .extend_rpc_modules({
            let impersonation = impersonation.clone();
            move |ctx| {
                ctx.registry
                    .eth_api()
                    .signers()
                    .write()
                    .push(Box::new(ImpersonatedSigner::new(impersonation.clone())));
                ctx.modules.merge_configured(
                    AnvilRpc::new(impersonation, ctx.pool().clone()).into_rpc(),
                )?;
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

#[cfg(test)]
async fn wait_for_receipt(client: &HttpClient, tx_hash: B256) -> Result<Value> {
    for _ in 0..50 {
        let receipt = client
            .request::<Option<Value>, _>("eth_getTransactionReceipt", rpc_params![tx_hash])
            .await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        sleep(Duration::from_millis(100)).await;
    }

    bail!("timed out waiting for receipt for {tx_hash}");
}

#[cfg(test)]
#[tokio::test]
async fn explicit_impersonation_allows_eth_send_transaction() -> Result<()> {
    let runtime = Runtime::test();
    let node_config = NodeConfig::test()
        .with_chain(DEV.clone())
        .dev()
        .with_rpc(RpcServerArgs::default().with_http());
    let impersonation = ImpersonationState::default();

    let NodeHandle { node, .. } = NodeBuilder::new(node_config)
        .testing_node(runtime)
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
            move |ctx| {
                ctx.registry
                    .eth_api()
                    .signers()
                    .write()
                    .push(Box::new(ImpersonatedSigner::new(impersonation.clone())));
                ctx.modules.merge_configured(
                    AnvilRpc::new(impersonation, ctx.pool().clone()).into_rpc(),
                )?;
                Ok(())
            }
        })
        .launch_with_debug_capabilities()
        .await?;

    let client = node
        .rpc_server_handles
        .rpc
        .http_client()
        .ok_or_eyre("http rpc client not available")?;

    let dev_accounts: Vec<Address> = client.request("eth_accounts", rpc_params![]).await?;
    let funder = *dev_accounts
        .first()
        .ok_or_eyre("no dev account available")?;
    let gas_price: u128 =
        client.request::<U256, _>("eth_gasPrice", rpc_params![]).await?.to::<u128>() +
            1_000_000_000u128;
    let target = Address::repeat_byte(0x11);
    let recipient = Address::repeat_byte(0x22);

    let funding_tx = TransactionRequest::default()
        .with_from(funder)
        .with_to(target)
        .with_gas_price(gas_price)
        .with_value(U256::from(1_000_000_000_000_000_000u64));
    let funding_hash: B256 = client
        .request("eth_sendTransaction", rpc_params![funding_tx])
        .await?;
    wait_for_receipt(&client, funding_hash).await?;

    client
        .request::<(), _>("hardhat_impersonateAccount", rpc_params![target])
        .await?;

    let impersonated_tx = TransactionRequest::default()
        .with_from(target)
        .with_to(recipient)
        .with_gas_price(gas_price)
        .with_value(U256::from(1));
    let impersonated_hash: B256 = client
        .request("eth_sendTransaction", rpc_params![impersonated_tx])
        .await?;
    wait_for_receipt(&client, impersonated_hash).await?;

    client
        .request::<(), _>("hardhat_stopImpersonatingAccount", rpc_params![target])
        .await?;

    let stopped_tx = TransactionRequest::default()
        .with_from(target)
        .with_to(recipient)
        .with_gas_price(gas_price)
        .with_value(U256::from(1));
    let err: ClientError = client
        .request::<B256, _>("eth_sendTransaction", rpc_params![stopped_tx])
        .await
        .expect_err("stopped impersonation should reject eth_sendTransaction");
    assert!(
        err.to_string().contains("unknown account"),
        "unexpected error after stop impersonating: {err}"
    );

    Ok(())
}
