mod anvil_api;
mod block_env;
mod block_source;
mod eth_builder;
mod evm;
mod impersonation;
mod mining;
mod pool;
mod snapshot;
mod state;
mod state_dump;
mod state_provider;
#[cfg(test)]
mod test_helpers;
mod time;

#[cfg(test)]
use alloy_network::{TransactionBuilder, TransactionResponse};
use alloy_primitives::B256;
#[cfg(test)]
use alloy_primitives::{Address, Bytes, U256};
#[cfg(test)]
use alloy_rpc_types_anvil::{Metadata, MineOptions, NodeInfo};
#[cfg(test)]
use alloy_rpc_types_eth::{state::StateOverridesBuilder, Block, TransactionRequest};
use anvil_api::{AnvilApiServer, AnvilContext, AnvilNodeConfig, AnvilRpc};
use block_env::BlockEnvOverrides;
use eth_builder::anvil_add_ons;
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
use reth_db_mem::MemoryDatabase;
use reth_engine_local::{LocalMinerHandle, MiningMode as LocalMiningMode};
use reth_ethereum::{
    chainspec::DEV,
    node::{
        builder::{
            components::{NoopConsensusBuilder, NoopNetworkBuilder},
            NodeBuilder, NodeHandle,
        },
        core::{
            args::{DatadirArgs, RpcServerArgs, StorageArgs},
            dirs::{DataDirPath, MaybePlatformPath},
            node_config::NodeConfig,
        },
        EthereumNode,
    },
    tasks::{RuntimeBuilder, RuntimeConfig},
};
#[cfg(test)]
use serde_json::Value;
use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use test_helpers::with_test_client;
use time::TimeManager;
#[cfg(test)]
use tokio::time::sleep;

use state::AnvilState;

#[tokio::main]
async fn main() -> Result<()> {
    let runtime = RuntimeBuilder::new(RuntimeConfig::default()).build()?;
    let datadir = MaybePlatformPath::<DataDirPath>::from(tempfile::tempdir()?.keep());
    let node_config = NodeConfig::new(DEV.clone())
        .with_unused_ports()
        .dev()
        .apply(|mut config| {
            config
                .engine
                .always_process_payload_attributes_on_canonical_head = true;
            config.engine.allow_unwind_canonical_header = true;
            config
        })
        .with_storage(StorageArgs { v2: false })
        .with_rpc(RpcServerArgs::default().with_http())
        .with_datadir_args(DatadirArgs {
            datadir,
            ..Default::default()
        });
    let impersonation = ImpersonationState::default();
    let mining = MiningController::default();
    let time = TimeManager::new(DEV.genesis_timestamp());
    let block_env = BlockEnvOverrides::default();
    let anvil_state = AnvilState::shared();
    let (local_miner, local_miner_control) = LocalMinerHandle::new();
    let anvil_context = AnvilContext::new(
        anvil_state.clone(),
        block_env.clone(),
        AnvilNodeConfig::new(DEV.clone(), B256::random()),
    );
    let trigger_stream = mining.trigger_stream();
    let NodeHandle {
        node,
        node_exit_future,
    } = NodeBuilder::new(node_config)
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
                    block_env: block_env.clone(),
                })
                .consensus(NoopConsensusBuilder),
        )
        .with_add_ons(anvil_add_ons(Arc::clone(&anvil_state)))
        .extend_rpc_modules({
            let impersonation = impersonation.clone();
            let mining = mining.clone();
            let time = time.clone();
            let anvil_context = anvil_context.clone();
            let local_miner = local_miner.clone();
            move |ctx| {
                ctx.registry
                    .eth_api()
                    .signers()
                    .write()
                    .push(Box::new(ImpersonatedSigner::new(impersonation.clone())));
                let rpc = AnvilRpc::new(
                    impersonation,
                    mining,
                    time,
                    anvil_context,
                    local_miner,
                    ctx.pool().clone(),
                    ctx.provider().clone(),
                    ctx.registry.eth_api().clone(),
                );
                ctx.modules
                    .merge_configured(AnvilApiServer::into_rpc(rpc))?;
                Ok(())
            }
        })
        .launch_with_debug_capabilities()
        .map_debug_payload_attributes(time.payload_attributes_hook(block_env))
        .with_mining_mode(LocalMiningMode::trigger(trigger_stream))
        .with_local_miner_control(local_miner_control)
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
mod tests {
    use super::*;
    use std::str::FromStr;

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

    async fn block_number(client: &HttpClient) -> Result<u64> {
        Ok(client
            .request::<U256, _>("eth_blockNumber", rpc_params![])
            .await?
            .to::<u64>())
    }

    async fn wait_for_block_number(client: &HttpClient, expected: u64) -> Result<()> {
        let mut last_seen = 0;

        for _ in 0..200 {
            let current = block_number(client).await?;
            if current >= expected {
                return Ok(());
            }

            last_seen = current;
            sleep(Duration::from_millis(100)).await;
        }

        bail!("timed out waiting for block {expected}, last seen {last_seen}");
    }

    async fn block_timestamp(client: &HttpClient, tag: impl Into<Value>) -> Result<u64> {
        let block: Value = client
            .request("eth_getBlockByNumber", rpc_params![tag.into(), false])
            .await?;
        Ok(U256::from_str(
            block["timestamp"]
                .as_str()
                .ok_or_eyre("missing block timestamp")?,
        )?
        .to::<u64>())
    }

    #[tokio::test]
    async fn explicit_impersonation_allows_eth_send_transaction() -> Result<()> {
        with_test_client(|client| async move {
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
        })
        .await
    }

    #[tokio::test]
    async fn set_automine_controls_transaction_mining() -> Result<()> {
        with_test_client(|client| async move {
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
        })
        .await
    }

    #[tokio::test]
    async fn anvil_mine_advances_requested_blocks() -> Result<()> {
        with_test_client(|client| async move {
            let initial_block = block_number(&client).await?;

            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            wait_for_block_number(&client, initial_block + 1).await?;

            client
                .request::<(), _>("hardhat_mine", rpc_params![U256::from(2)])
                .await?;
            wait_for_block_number(&client, initial_block + 3).await?;
            let pre_interval_timestamp = block_timestamp(&client, "latest").await?;

            client
                .request::<(), _>("anvil_mine", rpc_params![U256::from(2), U256::from(10)])
                .await?;
            wait_for_block_number(&client, initial_block + 5).await?;

            let first_interval_ts =
                block_timestamp(&client, format!("0x{:x}", initial_block + 4)).await?;
            let second_interval_ts =
                block_timestamp(&client, format!("0x{:x}", initial_block + 5)).await?;
            assert_eq!(first_interval_ts, pre_interval_timestamp + 10);
            assert_eq!(second_interval_ts, first_interval_ts + 10);

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn set_interval_mining_controls_block_production() -> Result<()> {
        with_test_client(|client| async move {
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
        })
        .await
    }

    #[tokio::test]
    async fn anvil_mine_detailed_returns_full_blocks() -> Result<()> {
        with_test_client(|client| async move {
            client
                .request::<(), _>("evm_setAutomine", rpc_params![false])
                .await?;

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
                .with_to(Address::repeat_byte(0x77))
                .with_gas_price(gas_price)
                .with_value(U256::from(1));
            let tx_hash: B256 = client
                .request("eth_sendTransaction", rpc_params![tx])
                .await?;

            let initial_block = block_number(&client).await?;
            let blocks: Vec<Block> = client
                .request(
                    "anvil_mine_detailed",
                    rpc_params![MineOptions::Options {
                        timestamp: None,
                        blocks: Some(2),
                    }],
                )
                .await?;

            assert_eq!(
                blocks.len(),
                2,
                "should return the requested number of blocks"
            );
            assert_eq!(blocks[0].number(), initial_block + 1);
            assert_eq!(blocks[1].number(), initial_block + 2);

            let first_block_txs = blocks[0]
                .transactions
                .as_transactions()
                .ok_or_eyre("anvil_mine_detailed should return full transactions")?;
            assert_eq!(
                first_block_txs.len(),
                1,
                "pending tx should be mined into first block"
            );
            assert_eq!(first_block_txs[0].tx_hash(), tx_hash);

            let second_block_txs = blocks[1]
                .transactions
                .as_transactions()
                .ok_or_eyre("anvil_mine_detailed should return full transactions")?;
            assert!(
                second_block_txs.is_empty(),
                "second block should be empty when there are no pending txs",
            );

            wait_for_receipt(&client, tx_hash).await?;

            let latest_timestamp = block_timestamp(&client, "latest").await?;
            let requested_timestamp = latest_timestamp + 15;
            let blocks: Vec<Block> = client
                .request(
                    "evm_mine_detailed",
                    rpc_params![MineOptions::Timestamp(Some(requested_timestamp))],
                )
                .await?;
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0].header.timestamp, requested_timestamp);

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_set_and_remove_block_timestamp_interval() -> Result<()> {
        with_test_client(|client| async move {
            let removed: bool = client
                .request("anvil_removeBlockTimestampInterval", rpc_params![])
                .await?;
            assert!(!removed, "should return false when no interval is set");

            client
                .request::<(), _>("anvil_setBlockTimestampInterval", rpc_params![10u64])
                .await?;

            let removed: bool = client
                .request("anvil_removeBlockTimestampInterval", rpc_params![])
                .await?;
            assert!(removed, "should return true when an interval was removed");

            let removed: bool = client
                .request("anvil_removeBlockTimestampInterval", rpc_params![])
                .await?;
            assert!(
                !removed,
                "should return false after interval was already removed"
            );

            client
                .request::<(), _>("anvil_setBlockTimestampInterval", rpc_params![10u64])
                .await?;

            client
                .request::<(), _>("anvil_setBlockTimestampInterval", rpc_params![20u64])
                .await?;

            let removed: bool = client
                .request("anvil_removeBlockTimestampInterval", rpc_params![])
                .await?;
            assert!(removed, "should return true after overwritten interval");

            let removed: bool = client
                .request("anvil_removeBlockTimestampInterval", rpc_params![])
                .await?;
            assert!(
                !removed,
                "should return false — second set overwrote, not stacked"
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_time_controls_are_visible_in_mined_blocks() -> Result<()> {
        with_test_client(|client| async move {
            let latest_timestamp = block_timestamp(&client, "latest").await?;

            let _increased: i64 = client
                .request("anvil_increaseTime", rpc_params![U256::from(30u64)])
                .await?;

            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            let after_increase = block_timestamp(&client, "latest").await?;
            assert!(
                after_increase >= latest_timestamp + 30,
                "increaseTime should move the next mined block forward by at least the requested amount",
            );

            let exact_timestamp = after_increase + 25;
            client
                .request::<(), _>("anvil_setNextBlockTimestamp", rpc_params![exact_timestamp])
                .await?;
            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            let after_exact = block_timestamp(&client, "latest").await?;
            assert_eq!(after_exact, exact_timestamp);

            let reset_timestamp = exact_timestamp + 40;
            let offset: u64 = client
                .request("anvil_setTime", rpc_params![reset_timestamp])
                .await?;
            assert!(
                offset <= 40,
                "setTime offset should not exceed requested jump"
            );
            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            let after_reset = block_timestamp(&client, "latest").await?;
            assert!(
                after_reset >= reset_timestamp,
                "setTime should move the time baseline forward without pinning an exact next-block timestamp",
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_set_balance_reflected_by_eth_get_balance() -> Result<()> {
        with_test_client(|client| async move {
            let target = Address::repeat_byte(0xBA);
            let new_balance = U256::from(42_000_000_000_000_000_000u128); // 42 ETH

            // Verify target starts with zero balance.
            let before: U256 = client
                .request("eth_getBalance", rpc_params![target, "latest"])
                .await?;
            assert_eq!(before, U256::ZERO, "target should start with zero balance");

            // Set balance via anvil_setBalance.
            client
                .request::<(), _>("anvil_setBalance", rpc_params![target, new_balance])
                .await?;

            // eth_getBalance should reflect the new value immediately.
            let after: U256 = client
                .request("eth_getBalance", rpc_params![target, "latest"])
                .await?;
            assert_eq!(
                after, new_balance,
                "eth_getBalance should return the value set by anvil_setBalance"
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_set_balance_is_visible_to_stock_eth_call() -> Result<()> {
        with_test_client(|client| async move {
            let target = Address::repeat_byte(0xBA);
            let contract = Address::repeat_byte(0xCC);
            let new_balance = U256::from(42_000_000_000_000_000_000u128);

            client
                .request::<(), _>("anvil_setBalance", rpc_params![target, new_balance])
                .await?;

            // Bytecode: PUSH20 <target> BALANCE PUSH1 0x00 MSTORE PUSH1 0x20 PUSH1 0x00 RETURN
            let mut bytecode = Vec::with_capacity(30);
            bytecode.push(0x73);
            bytecode.extend_from_slice(target.as_slice());
            bytecode.extend_from_slice(&[0x31, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);

            let state_override = StateOverridesBuilder::default()
                .with_code(contract, Bytes::from(bytecode))
                .build();
            let call = TransactionRequest::default().with_to(contract);

            let result: Bytes = client
                .request("eth_call", rpc_params![call, "latest", state_override])
                .await?;

            assert_eq!(
                U256::from_be_slice(result.as_ref()),
                new_balance,
                "eth_call should see the balance set by anvil_setBalance"
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_add_balance_accumulates_and_is_visible_to_reads() -> Result<()> {
        with_test_client(|client| async move {
            let target = Address::repeat_byte(0xAD);
            let contract = Address::repeat_byte(0xCE);
            let first = U256::from(7u64);
            let second = U256::from(9u64);
            let expected = first + second;

            client
                .request::<(), _>("anvil_addBalance", rpc_params![target, first])
                .await?;
            client
                .request::<(), _>("anvil_addBalance", rpc_params![target, second])
                .await?;

            let balance: U256 = client
                .request("eth_getBalance", rpc_params![target, "latest"])
                .await?;
            assert_eq!(
                balance, expected,
                "eth_getBalance should reflect the accumulated balance"
            );

            let mut bytecode = Vec::with_capacity(30);
            bytecode.push(0x73);
            bytecode.extend_from_slice(target.as_slice());
            bytecode.extend_from_slice(&[0x31, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);

            let state_override = StateOverridesBuilder::default()
                .with_code(contract, Bytes::from(bytecode))
                .build();
            let call = TransactionRequest::default().with_to(contract);
            let result: Bytes = client
                .request("eth_call", rpc_params![call, "latest", state_override])
                .await?;

            assert_eq!(
                U256::from_be_slice(result.as_ref()),
                expected,
                "eth_call should see the accumulated balance"
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_deal_erc20_is_visible_to_eth_call() -> Result<()> {
        with_test_client(|client| async move {
            let token = Address::repeat_byte(0xD0);
            let target = Address::repeat_byte(0xD1);
            let amount = U256::from(500u64);
            let bytecode = Bytes::from_static(&[
                0x60, 0x00, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
            ]);

            client
                .request::<(), _>("anvil_setCode", rpc_params![token, bytecode])
                .await?;
            client
                .request::<(), _>("anvil_dealERC20", rpc_params![target, token, amount])
                .await?;

            let mut calldata = Vec::with_capacity(4 + 32);
            calldata.extend_from_slice(&[0x70, 0xa0, 0x82, 0x31]);
            calldata.extend_from_slice(&[0u8; 12]);
            calldata.extend_from_slice(target.as_slice());

            let call = TransactionRequest::default()
                .with_to(token)
                .with_input(Bytes::from(calldata));
            let result: Bytes = client
                .request("eth_call", rpc_params![call, "latest"])
                .await?;

            assert_eq!(
                U256::from_be_slice(result.as_ref()),
                amount,
                "eth_call should see the ERC20 balance override"
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_set_erc20_allowance_is_visible_to_eth_call() -> Result<()> {
        with_test_client(|client| async move {
            let token = Address::repeat_byte(0xD2);
            let owner = Address::repeat_byte(0xD3);
            let spender = Address::repeat_byte(0xD4);
            let amount = U256::from(777u64);
            let bytecode = Bytes::from_static(&[
                0x60, 0x00, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
            ]);

            client
                .request::<(), _>("anvil_setCode", rpc_params![token, bytecode])
                .await?;
            client
                .request::<(), _>(
                    "anvil_setERC20Allowance",
                    rpc_params![owner, spender, token, amount],
                )
                .await?;

            let mut calldata = Vec::with_capacity(4 + 32 + 32);
            calldata.extend_from_slice(&[0xdd, 0x62, 0xed, 0x3e]);
            calldata.extend_from_slice(&[0u8; 12]);
            calldata.extend_from_slice(owner.as_slice());
            calldata.extend_from_slice(&[0u8; 12]);
            calldata.extend_from_slice(spender.as_slice());

            let call = TransactionRequest::default()
                .with_to(token)
                .with_input(Bytes::from(calldata));
            let result: Bytes = client
                .request("eth_call", rpc_params![call, "latest"])
                .await?;

            assert_eq!(
                U256::from_be_slice(result.as_ref()),
                amount,
                "eth_call should see the ERC20 allowance override"
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_set_nonce_reflected_by_eth_get_transaction_count() -> Result<()> {
        with_test_client(|client| async move {
            let target = Address::repeat_byte(0xAB);
            let new_nonce = U256::from(7u64);

            let before: U256 = client
                .request("eth_getTransactionCount", rpc_params![target, "latest"])
                .await?;
            assert_eq!(before, U256::ZERO, "target should start with zero nonce");

            client
                .request::<(), _>("anvil_setNonce", rpc_params![target, new_nonce])
                .await?;

            let after: U256 = client
                .request("eth_getTransactionCount", rpc_params![target, "latest"])
                .await?;
            assert_eq!(
                after, new_nonce,
                "eth_getTransactionCount should see the override"
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_set_code_is_visible_to_eth_get_code_and_eth_call() -> Result<()> {
        with_test_client(|client| async move {
            let contract = Address::repeat_byte(0xCD);
            let bytecode =
                Bytes::from_static(&[0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);

            let before: Bytes = client
                .request("eth_getCode", rpc_params![contract, "latest"])
                .await?;
            assert!(before.is_empty(), "target should start without code");

            client
                .request::<(), _>("anvil_setCode", rpc_params![contract, bytecode.clone()])
                .await?;

            let after: Bytes = client
                .request("eth_getCode", rpc_params![contract, "latest"])
                .await?;
            assert_eq!(
                after, bytecode,
                "eth_getCode should see the overridden code"
            );

            let call = TransactionRequest::default().with_to(contract);
            let result: Bytes = client
                .request("eth_call", rpc_params![call, "latest"])
                .await?;

            assert_eq!(
                U256::from_be_slice(result.as_ref()),
                U256::from(42u64),
                "eth_call should execute the overridden code"
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_set_storage_at_is_visible_to_eth_get_storage_at_and_eth_call() -> Result<()> {
        with_test_client(|client| async move {
            let contract = Address::repeat_byte(0xCE);
            let slot = U256::ZERO;
            let value = B256::from(U256::from(0xBEEFu64));
            let bytecode = Bytes::from_static(&[
                0x60, 0x00, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
            ]);

            client
                .request::<(), _>("anvil_setCode", rpc_params![contract, bytecode])
                .await?;
            let updated: bool = client
                .request("anvil_setStorageAt", rpc_params![contract, slot, value])
                .await?;
            assert!(updated, "anvil_setStorageAt should return true");

            let storage: B256 = client
                .request("eth_getStorageAt", rpc_params![contract, slot, "latest"])
                .await?;
            assert_eq!(
                storage, value,
                "eth_getStorageAt should see the overridden storage"
            );

            let call = TransactionRequest::default().with_to(contract);
            let result: Bytes = client
                .request("eth_call", rpc_params![call, "latest"])
                .await?;

            assert_eq!(
                B256::from_slice(result.as_ref()),
                value,
                "eth_call should load the overridden storage value"
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_dump_and_load_state_round_trip() -> Result<()> {
        with_test_client(|client| async move {
            let account = Address::repeat_byte(0xA1);
            let contract = Address::repeat_byte(0xC1);
            let balance = U256::from(123_456u64);
            let nonce = U256::from(7u64);
            let slot = U256::ZERO;
            let storage = B256::from(U256::from(0xBEEFu64));
            let code =
                Bytes::from_static(&[0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);

            client
                .request::<(), _>("anvil_setBalance", rpc_params![account, balance])
                .await?;
            client
                .request::<(), _>("anvil_setNonce", rpc_params![account, nonce])
                .await?;
            client
                .request::<(), _>("anvil_setCode", rpc_params![contract, code.clone()])
                .await?;
            client
                .request::<bool, _>("anvil_setStorageAt", rpc_params![contract, slot, storage])
                .await?;

            let dump: Bytes = client.request("anvil_dumpState", rpc_params![]).await?;
            assert!(
                !dump.is_empty(),
                "dumpState should return gzipped state bytes"
            );

            let replacement_code =
                Bytes::from_static(&[0x60, 0x07, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
            client
                .request::<(), _>("anvil_setBalance", rpc_params![account, U256::from(1u64)])
                .await?;
            client
                .request::<(), _>("anvil_setNonce", rpc_params![account, U256::from(1u64)])
                .await?;
            client
                .request::<(), _>("anvil_setCode", rpc_params![contract, replacement_code])
                .await?;
            client
                .request::<bool, _>(
                    "anvil_setStorageAt",
                    rpc_params![contract, slot, B256::from(U256::from(1u64))],
                )
                .await?;

            let loaded: bool = client.request("anvil_loadState", rpc_params![dump]).await?;
            assert!(loaded, "loadState should return true");

            let loaded_balance: U256 = client
                .request("eth_getBalance", rpc_params![account, "latest"])
                .await?;
            assert_eq!(loaded_balance, balance);

            let loaded_nonce: U256 = client
                .request("eth_getTransactionCount", rpc_params![account, "latest"])
                .await?;
            assert_eq!(loaded_nonce, nonce);

            let loaded_code: Bytes = client
                .request("eth_getCode", rpc_params![contract, "latest"])
                .await?;
            assert_eq!(loaded_code, code);

            let loaded_storage: B256 = client
                .request("eth_getStorageAt", rpc_params![contract, slot, "latest"])
                .await?;
            assert_eq!(loaded_storage, storage);

            let result: Bytes = client
                .request(
                    "eth_call",
                    rpc_params![TransactionRequest::default().with_to(contract), "latest"],
                )
                .await?;
            assert_eq!(U256::from_be_slice(result.as_ref()), U256::from(42u64));

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_snapshot_and_revert_restore_overrides_and_metadata() -> Result<()> {
        with_test_client(|client| async move {
            let account = Address::repeat_byte(0x5A);
            let original_balance = U256::from(123u64);
            let original_gas_limit = U256::from(25_000_000u64);
            let replacement_gas_limit = U256::from(30_000_000u64);

            client
                .request::<(), _>("anvil_setBalance", rpc_params![account, original_balance])
                .await?;
            client
                .request::<bool, _>("anvil_setBlockGasLimit", rpc_params![original_gas_limit])
                .await?;

            let snapshot: U256 = client.request("evm_snapshot", rpc_params![]).await?;
            let snapshot_block_number = block_number(&client).await?;
            let snapshot_block = get_block(&client, "latest").await?;
            let snapshot_block_hash = B256::from_str(
                snapshot_block["hash"]
                    .as_str()
                    .ok_or_eyre("missing snapshot hash")?,
            )?;

            let metadata: Metadata = client.request("anvil_metadata", rpc_params![]).await?;
            assert_eq!(
                metadata.snapshots.get(&snapshot),
                Some(&(snapshot_block_number, snapshot_block_hash))
            );

            client
                .request::<(), _>("anvil_setBalance", rpc_params![account, U256::from(1u64)])
                .await?;
            client
                .request::<bool, _>("evm_setBlockGasLimit", rpc_params![replacement_gas_limit])
                .await?;

            let reverted: bool = client.request("evm_revert", rpc_params![snapshot]).await?;
            assert!(reverted, "revert should return true for a known snapshot");

            let balance: U256 = client
                .request("eth_getBalance", rpc_params![account, "latest"])
                .await?;
            assert_eq!(balance, original_balance);

            let second_revert: bool = client
                .request("anvil_revert", rpc_params![snapshot])
                .await?;
            assert!(
                !second_revert,
                "snapshot ids should be invalidated after a successful revert"
            );

            let metadata: Metadata = client.request("anvil_metadata", rpc_params![]).await?;
            assert!(!metadata.snapshots.contains_key(&snapshot));

            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            let mined = get_block(&client, "latest").await?;
            let gas_limit =
                U256::from_str(mined["gasLimit"].as_str().ok_or_eyre("missing gasLimit")?)?;
            assert_eq!(gas_limit, original_gas_limit);

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_revert_restores_head_and_mines_from_snapshot() -> Result<()> {
        with_test_client(|client| async move {
            let initial_block = block_number(&client).await?;
            let snapshot: U256 = client.request("anvil_snapshot", rpc_params![]).await?;

            client
                .request::<(), _>("anvil_mine", rpc_params![U256::from(2u64)])
                .await?;
            wait_for_block_number(&client, initial_block + 2).await?;
            assert_eq!(block_number(&client).await?, initial_block + 2);

            let reverted: bool = client
                .request("anvil_revert", rpc_params![snapshot])
                .await?;
            assert!(reverted);
            assert_eq!(block_number(&client).await?, initial_block);

            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            wait_for_block_number(&client, initial_block + 1).await?;
            assert_eq!(
                block_number(&client).await?,
                initial_block + 1,
                "mining after revert should extend the restored snapshot head"
            );

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_node_info_and_metadata_follow_latest_head() -> Result<()> {
        with_test_client(|client| async move {
            let expected_block_number = block_number(&client).await?;
            let expected_gas_price: u128 = client
                .request::<U256, _>("eth_gasPrice", rpc_params![])
                .await?
                .to::<u128>();
            let latest_block: Value = client
                .request("eth_getBlockByNumber", rpc_params!["latest", false])
                .await?;
            let expected_hash = B256::from_str(
                latest_block["hash"]
                    .as_str()
                    .ok_or_eyre("missing latest hash")?,
            )?;
            let expected_timestamp = U256::from_str(
                latest_block["timestamp"]
                    .as_str()
                    .ok_or_eyre("missing latest timestamp")?,
            )?
            .to::<u64>();

            let node_info: NodeInfo = client.request("anvil_nodeInfo", rpc_params![]).await?;
            let metadata: Metadata = client.request("anvil_metadata", rpc_params![]).await?;
            let hardhat_metadata: Metadata =
                client.request("hardhat_metadata", rpc_params![]).await?;

            assert_eq!(node_info.current_block_number, expected_block_number);
            assert_eq!(node_info.current_block_timestamp, expected_timestamp);
            assert_eq!(node_info.current_block_hash, expected_hash);
            assert_eq!(node_info.transaction_order, "fees");
            assert_eq!(node_info.environment.chain_id, metadata.chain_id);
            assert_eq!(node_info.environment.gas_price, expected_gas_price);
            assert_eq!(metadata.latest_block_number, expected_block_number);
            assert_eq!(metadata.latest_block_hash, expected_hash);
            assert_eq!(
                metadata.client_version,
                format!("anvil-reth/v{}", env!("CARGO_PKG_VERSION"))
            );
            assert_eq!(
                metadata.client_semver.as_deref(),
                Some(env!("CARGO_PKG_VERSION"))
            );
            assert!(metadata.snapshots.is_empty());
            assert_eq!(hardhat_metadata, metadata);

            client.request::<(), _>("anvil_mine", rpc_params![]).await?;

            let mined_number = block_number(&client).await?;
            let mined_info: NodeInfo = client.request("anvil_nodeInfo", rpc_params![]).await?;
            let mined_metadata: Metadata = client.request("anvil_metadata", rpc_params![]).await?;

            assert_eq!(mined_info.current_block_number, mined_number);
            assert_eq!(mined_metadata.latest_block_number, mined_number);
            assert_eq!(
                mined_info.current_block_hash,
                mined_metadata.latest_block_hash
            );

            Ok(())
        })
        .await
    }

    async fn get_block(client: &HttpClient, tag: impl Into<Value>) -> Result<Value> {
        Ok(client
            .request("eth_getBlockByNumber", rpc_params![tag.into(), false])
            .await?)
    }

    #[tokio::test]
    async fn set_block_gas_limit_accepts_anvil_and_evm_namespaces() -> Result<()> {
        with_test_client(|client| async move {
            for (method, custom_limit) in [
                ("evm_setBlockGasLimit", U256::from(20_000_000u64)),
                ("anvil_setBlockGasLimit", U256::from(21_000_000u64)),
            ] {
                let ok: bool = client.request(method, rpc_params![custom_limit]).await?;
                assert!(ok, "{method} should return true");

                // Mine first block and verify gas limit.
                client.request::<(), _>("anvil_mine", rpc_params![]).await?;
                let block1 = get_block(&client, "latest").await?;
                let gas_limit_1 =
                    U256::from_str(block1["gasLimit"].as_str().ok_or_eyre("missing gasLimit")?)?;
                assert_eq!(
                    gas_limit_1, custom_limit,
                    "{method} should affect the first mined block"
                );

                // Mine second block — should persist.
                client.request::<(), _>("anvil_mine", rpc_params![]).await?;
                let block2 = get_block(&client, "latest").await?;
                let gas_limit_2 =
                    U256::from_str(block2["gasLimit"].as_str().ok_or_eyre("missing gasLimit")?)?;
                assert_eq!(
                    gas_limit_2, custom_limit,
                    "{method} gas limit should persist across blocks"
                );
            }

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_set_coinbase_persists_across_blocks() -> Result<()> {
        with_test_client(|client| async move {
            let coinbase = Address::repeat_byte(0xCB);

            client
                .request::<(), _>("anvil_setCoinbase", rpc_params![coinbase])
                .await?;

            // Mine first block and verify coinbase.
            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            let block1 = get_block(&client, "latest").await?;
            let miner_1 = Address::from_str(block1["miner"].as_str().ok_or_eyre("missing miner")?)?;
            assert_eq!(
                miner_1, coinbase,
                "first mined block should use the overridden coinbase"
            );

            // Mine second block — should persist.
            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            let block2 = get_block(&client, "latest").await?;
            let miner_2 = Address::from_str(block2["miner"].as_str().ok_or_eyre("missing miner")?)?;
            assert_eq!(miner_2, coinbase, "coinbase should persist across blocks");

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn anvil_set_next_block_base_fee_per_gas_is_consumed_once() -> Result<()> {
        with_test_client(|client| async move {
            let custom_base_fee = U256::from(42_000_000_000u64); // 42 gwei

            client
                .request::<(), _>(
                    "anvil_setNextBlockBaseFeePerGas",
                    rpc_params![custom_base_fee],
                )
                .await?;

            // Mine the target block.
            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            let target_block = get_block(&client, "latest").await?;
            let base_fee = U256::from_str(
                target_block["baseFeePerGas"]
                    .as_str()
                    .ok_or_eyre("missing baseFeePerGas")?,
            )?;
            assert_eq!(
                base_fee, custom_base_fee,
                "next mined block should use the overridden base fee"
            );

            // Mine another block — should NOT use the override (consumed).
            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            let after_block = get_block(&client, "latest").await?;
            let after_base_fee = U256::from_str(
                after_block["baseFeePerGas"]
                    .as_str()
                    .ok_or_eyre("missing baseFeePerGas")?,
            )?;
            assert_ne!(
                after_base_fee, custom_base_fee,
                "base fee override should be consumed after one block"
            );

            Ok(())
        })
        .await
    }
}
