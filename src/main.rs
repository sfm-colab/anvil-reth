mod anvil_api;
mod evm;
mod impersonation;
mod mining;
mod pool;

#[cfg(test)]
use alloy_network::TransactionBuilder;
#[cfg(test)]
use alloy_primitives::{Address, B256, U256};
#[cfg(test)]
use alloy_rpc_types_eth::TransactionRequest;
use anvil_api::{AnvilApiServer, AnvilRpc};
use evm::AnvilExecutorBuilder;
use eyre::Result;
#[cfg(test)]
use eyre::{bail, OptionExt};
use impersonation::{ImpersonatedSigner, ImpersonationState};
#[cfg(test)]
use jsonrpsee::{
    core::{client::ClientT, ClientError},
    http_client::HttpClient,
    rpc_params,
};
use mining::{run_automine_task, run_interval_mining_task, MiningController};
use pool::AnvilPoolBuilder;
use reth_engine_local::MiningMode as LocalMiningMode;
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
    let mining = MiningController::default();
    let trigger_stream = mining.trigger_stream();
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
                        ctx.pool().clone(),
                        ctx.provider().clone(),
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
async fn assert_no_receipt(client: &HttpClient, tx_hash: B256, attempts: usize) -> Result<()> {
    for _ in 0..attempts {
        let receipt = client
            .request::<Option<Value>, _>("eth_getTransactionReceipt", rpc_params![tx_hash])
            .await?;
        if receipt.is_some() {
            bail!("unexpected receipt for {tx_hash}");
        }
        sleep(Duration::from_millis(100)).await;
    }

    Ok(())
}

#[cfg(test)]
async fn block_number(client: &HttpClient) -> Result<u64> {
    Ok(client
        .request::<U256, _>("eth_blockNumber", rpc_params![])
        .await?
        .to::<u64>())
}

#[cfg(test)]
async fn wait_for_block_number(client: &HttpClient, expected: u64) -> Result<()> {
    let mut last_seen = 0;

    for _ in 0..100 {
        let current = block_number(client).await?;
        if current >= expected {
            return Ok(());
        }

        last_seen = current;
        sleep(Duration::from_millis(100)).await;
    }

    bail!("timed out waiting for block {expected}, last seen {last_seen}");
}

#[cfg(test)]
#[tokio::test]
async fn explicit_impersonation_allows_eth_send_transaction() -> Result<()> {
    let runtime = Runtime::test();
    let node_config = NodeConfig::test()
        .with_chain(DEV.clone())
        .dev()
        .with_rpc(RpcServerArgs::default().with_unused_ports().with_http());
    let impersonation = ImpersonationState::default();
    let mining = MiningController::default();
    let trigger_stream = mining.trigger_stream();

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
                        ctx.pool().clone(),
                        ctx.provider().clone(),
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

    let dev_accounts: Vec<Address> = client.request("eth_accounts", rpc_params![]).await?;
    let funder = *dev_accounts
        .first()
        .ok_or_eyre("no dev account available")?;
    let gas_price: u128 = client
        .request::<U256, _>("eth_gasPrice", rpc_params![])
        .await?
        .to::<u128>()
        + 1_000_000_000u128;
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

#[cfg(test)]
#[tokio::test]
async fn set_automine_controls_transaction_mining() -> Result<()> {
    let runtime = Runtime::test();
    let node_config = NodeConfig::test()
        .with_chain(DEV.clone())
        .dev()
        .with_rpc(RpcServerArgs::default().with_unused_ports().with_http());
    let impersonation = ImpersonationState::default();
    let mining = MiningController::default();
    let trigger_stream = mining.trigger_stream();

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
                        ctx.pool().clone(),
                        ctx.provider().clone(),
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

    let dev_accounts: Vec<Address> = client.request("eth_accounts", rpc_params![]).await?;
    let funder = *dev_accounts
        .first()
        .ok_or_eyre("no dev account available")?;
    let gas_price: u128 = client
        .request::<U256, _>("eth_gasPrice", rpc_params![])
        .await?
        .to::<u128>()
        + 1_000_000_000u128;
    let initial_block = block_number(&client).await?;
    let enabled: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
    assert!(enabled, "automine should be enabled by default");
    let interval: Option<u64> = client
        .request("anvil_getIntervalMining", rpc_params![])
        .await?;
    assert_eq!(interval, None, "interval mining should be unset by default");

    client
        .request::<(), _>("evm_setAutomine", rpc_params![false])
        .await?;
    let enabled: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
    assert!(
        !enabled,
        "automine should be disabled after evm_setAutomine(false)"
    );
    let interval: Option<u64> = client
        .request("anvil_getIntervalMining", rpc_params![])
        .await?;
    assert_eq!(
        interval, None,
        "manual mode should not report interval mining"
    );

    let tx = TransactionRequest::default()
        .with_from(funder)
        .with_to(Address::repeat_byte(0x33))
        .with_gas_price(gas_price)
        .with_value(U256::from(1));
    let tx_hash: B256 = client
        .request("eth_sendTransaction", rpc_params![tx])
        .await?;

    assert_no_receipt(&client, tx_hash, 5).await?;
    assert_eq!(block_number(&client).await?, initial_block);

    client
        .request::<(), _>("anvil_setAutomine", rpc_params![true])
        .await?;
    let enabled: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
    assert!(
        enabled,
        "automine should be enabled after anvil_setAutomine(true)"
    );
    let interval: Option<u64> = client
        .request("anvil_getIntervalMining", rpc_params![])
        .await?;
    assert_eq!(interval, None, "automine should clear interval mining");

    wait_for_receipt(&client, tx_hash).await?;
    assert_eq!(block_number(&client).await?, initial_block + 1);

    Ok(())
}

#[cfg(test)]
#[tokio::test]
async fn anvil_mine_advances_requested_blocks() -> Result<()> {
    let runtime = Runtime::test();
    let node_config = NodeConfig::test()
        .with_chain(DEV.clone())
        .dev()
        .with_rpc(RpcServerArgs::default().with_unused_ports().with_http());
    let impersonation = ImpersonationState::default();
    let mining = MiningController::default();
    let trigger_stream = mining.trigger_stream();

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
                        ctx.pool().clone(),
                        ctx.provider().clone(),
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
    let initial_block = block_number(&client).await?;

    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    wait_for_block_number(&client, initial_block + 1).await?;

    client
        .request::<(), _>("hardhat_mine", rpc_params![U256::from(2)])
        .await?;
    wait_for_block_number(&client, initial_block + 3).await?;

    let err: ClientError = client
        .request::<(), _>("anvil_mine", rpc_params![U256::from(1), U256::from(1)])
        .await
        .expect_err("non-zero interval should fail until timestamp controls exist");
    assert!(
        err.to_string().contains("interval is not supported yet"),
        "unexpected error: {err:?}",
    );

    Ok(())
}

#[cfg(test)]
#[tokio::test]
async fn set_interval_mining_controls_block_production() -> Result<()> {
    let runtime = Runtime::test();
    let node_config = NodeConfig::test()
        .with_chain(DEV.clone())
        .dev()
        .with_rpc(RpcServerArgs::default().with_unused_ports().with_http());
    let impersonation = ImpersonationState::default();
    let mining = MiningController::default();
    let trigger_stream = mining.trigger_stream();

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
                        ctx.pool().clone(),
                        ctx.provider().clone(),
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

    let initial_block = block_number(&client).await?;

    client
        .request::<(), _>("evm_setIntervalMining", rpc_params![2u64])
        .await?;
    let enabled: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
    assert!(!enabled, "automine should be false in interval mining mode");
    let interval: Option<u64> = client
        .request("anvil_getIntervalMining", rpc_params![])
        .await?;
    assert_eq!(
        interval,
        Some(2),
        "interval mining should report the configured value"
    );

    client
        .request::<(), _>("evm_setAutomine", rpc_params![false])
        .await?;
    let interval: Option<u64> = client
        .request("anvil_getIntervalMining", rpc_params![])
        .await?;
    assert_eq!(
        interval,
        Some(2),
        "disabling automine should not clear interval mining"
    );

    wait_for_block_number(&client, initial_block + 1).await?;

    let dev_accounts: Vec<Address> = client.request("eth_accounts", rpc_params![]).await?;
    let funder = *dev_accounts
        .first()
        .ok_or_eyre("no dev account available")?;
    let gas_price: u128 = client
        .request::<U256, _>("eth_gasPrice", rpc_params![])
        .await?
        .to::<u128>()
        + 1_000_000_000u128;
    let tx = TransactionRequest::default()
        .with_from(funder)
        .with_to(Address::repeat_byte(0x44))
        .with_gas_price(gas_price)
        .with_value(U256::from(1));
    let tx_hash: B256 = client
        .request("eth_sendTransaction", rpc_params![tx])
        .await?;

    wait_for_receipt(&client, tx_hash).await?;

    client
        .request::<(), _>("anvil_setIntervalMining", rpc_params![0u64])
        .await?;
    let enabled: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
    assert!(!enabled, "manual mode should not report automine");
    let interval: Option<u64> = client
        .request("anvil_getIntervalMining", rpc_params![])
        .await?;
    assert_eq!(
        interval, None,
        "zero interval should disable interval mining"
    );
    let tx = TransactionRequest::default()
        .with_from(funder)
        .with_to(Address::repeat_byte(0x55))
        .with_gas_price(gas_price)
        .with_value(U256::from(1));
    let tx_hash: B256 = client
        .request("eth_sendTransaction", rpc_params![tx])
        .await?;
    assert_no_receipt(&client, tx_hash, 10).await?;
    let block_after_manual = block_number(&client).await?;
    sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        block_number(&client).await?,
        block_after_manual,
        "manual mode should not keep producing interval blocks",
    );

    let tx = TransactionRequest::default()
        .with_from(funder)
        .with_to(Address::repeat_byte(0x66))
        .with_gas_price(gas_price)
        .with_value(U256::from(1));
    let tx_hash: B256 = client
        .request("eth_sendTransaction", rpc_params![tx])
        .await?;
    assert_no_receipt(&client, tx_hash, 5).await?;

    client
        .request::<(), _>("anvil_mine", rpc_params![U256::from(1), U256::ZERO])
        .await?;
    wait_for_receipt(&client, tx_hash).await?;

    Ok(())
}
