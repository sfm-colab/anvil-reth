use crate::{block_env::BlockEnvSnapshot, state::AnvilState, time::TimeSnapshot};
use alloy_consensus::Header;
use alloy_eips::BlockHashOrNumber;
use alloy_primitives::{B256, U256};
use parking_lot::RwLock;
use reth_chain_state::NewCanonicalChain;
use reth_db_mem::MemoryDatabase;
use reth_ethereum::node::{builder::NodeTypesWithDBAdapter, EthereumNode};
use reth_primitives_traits::SealedHeader;
use reth_provider::providers::BlockchainProvider;
use reth_storage_api::{
    errors::provider::{ProviderError, ProviderResult},
    BlockExecutionWriter, BlockNumReader, DBProvider, DatabaseProviderFactory, HeaderProvider,
};
use std::{collections::BTreeMap, sync::Arc};

type AnvilBlockchainProvider =
    BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, Arc<MemoryDatabase>>>;

/// Captured Anvil-local state for a snapshot id.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub block_number: u64,
    pub block_hash: B256,
    pub anvil_state: AnvilState,
    pub time: TimeSnapshot,
    pub block_env: BlockEnvSnapshot,
}

impl Snapshot {
    pub fn new(
        block_number: u64,
        block_hash: B256,
        anvil_state: AnvilState,
        time: TimeSnapshot,
        block_env: BlockEnvSnapshot,
    ) -> Self {
        Self {
            block_number,
            block_hash,
            anvil_state,
            time,
            block_env,
        }
    }
}

/// Tracks Anvil snapshot ids and their captured state.
#[derive(Clone, Debug, Default)]
pub struct SnapshotManager {
    inner: Arc<RwLock<Snapshots>>,
}

#[derive(Debug)]
struct Snapshots {
    next_id: U256,
    snapshots: BTreeMap<U256, Snapshot>,
}

impl Default for Snapshots {
    fn default() -> Self {
        Self {
            next_id: U256::from(1u64),
            snapshots: BTreeMap::new(),
        }
    }
}

impl SnapshotManager {
    pub fn insert(&self, snapshot: Snapshot) -> U256 {
        let mut inner = self.inner.write();
        let id = inner.next_id;
        inner.next_id += U256::from(1u64);
        inner.snapshots.insert(id, snapshot);
        id
    }

    pub fn get(&self, id: U256) -> Option<Snapshot> {
        self.inner.read().snapshots.get(&id).cloned()
    }

    pub fn invalidate_from(&self, id: U256) {
        let mut inner = self.inner.write();
        inner.snapshots.retain(|snapshot_id, _| *snapshot_id < id);
    }

    pub fn metadata(&self) -> BTreeMap<U256, (u64, B256)> {
        self.inner
            .read()
            .snapshots
            .iter()
            .map(|(id, snapshot)| (*id, (snapshot.block_number, snapshot.block_hash)))
            .collect()
    }
}

/// Provider operations needed to restore a snapshot block head.
pub trait ChainSnapshotProvider {
    type Header;

    fn snapshot_header(&self, snapshot: &Snapshot) -> ProviderResult<SealedHeader<Self::Header>>;

    fn finalize_snapshot_revert(
        &self,
        snapshot: &Snapshot,
        header: SealedHeader<Self::Header>,
    ) -> ProviderResult<()>;
}

impl ChainSnapshotProvider for AnvilBlockchainProvider {
    type Header = Header;

    fn snapshot_header(&self, snapshot: &Snapshot) -> ProviderResult<SealedHeader<Self::Header>> {
        self.sealed_header(snapshot.block_number)?
            .filter(|header| header.hash() == snapshot.block_hash)
            .ok_or_else(|| {
                ProviderError::HeaderNotFound(BlockHashOrNumber::Hash(snapshot.block_hash))
            })
    }

    fn finalize_snapshot_revert(
        &self,
        snapshot: &Snapshot,
        header: SealedHeader<Self::Header>,
    ) -> ProviderResult<()> {
        let last_block_number = self.last_block_number()?;
        if last_block_number > snapshot.block_number {
            let provider = self.database_provider_rw()?;
            provider.remove_block_and_execution_above(snapshot.block_number)?;
            provider.commit()?;
        }

        let in_memory = self.canonical_in_memory_state();
        let snapshot_is_persisted = last_block_number >= snapshot.block_number;
        let old_blocks = in_memory
            .canonical_chain()
            .filter(|block| {
                if snapshot_is_persisted {
                    block.number() >= snapshot.block_number
                } else {
                    block.number() > snapshot.block_number
                }
            })
            .map(|block| block.block())
            .collect();
        in_memory.update_chain(NewCanonicalChain::Reorg {
            new: Vec::new(),
            old: old_blocks,
        });

        in_memory.set_canonical_head(header.clone());
        in_memory.set_safe(header.clone());
        in_memory.set_finalized(header.clone());

        Ok(())
    }
}
