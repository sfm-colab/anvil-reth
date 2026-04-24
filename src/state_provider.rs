use crate::state::SharedAnvilState;
use alloy_primitives::{Address, StorageKey, StorageValue, B256};
use reth_primitives_traits::{Account, Bytecode};
use reth_storage_api::{
    errors::provider::ProviderResult, AccountReader, BlockHashReader, BytecodeReader,
    HashedPostStateProvider, StateProofProvider, StateProvider, StateProviderBox,
    StateRootProvider, StorageRootProvider,
};
use reth_trie::{
    updates::TrieUpdates, AccountProof, ExecutionWitnessMode, HashedPostState, HashedStorage,
    MultiProof, MultiProofTargets, StorageMultiProof, StorageProof, TrieInput,
};
use revm_database::BundleState;
use std::fmt::{self, Debug, Formatter};

/// State provider wrapper that overlays local Anvil-owned account state on top of the base Reth
/// provider returned through `EthApi` load-state helpers.
pub struct AnvilStateProvider {
    state: SharedAnvilState,
    inner: StateProviderBox,
}

impl Debug for AnvilStateProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilStateProvider").finish_non_exhaustive()
    }
}

impl AnvilStateProvider {
    pub fn new(state: SharedAnvilState, inner: StateProviderBox) -> Self {
        Self { state, inner }
    }
}

impl AsRef<dyn StateProvider + Send + 'static> for AnvilStateProvider {
    fn as_ref(&self) -> &(dyn StateProvider + Send + 'static) {
        self.inner.as_ref()
    }
}

impl AccountReader for AnvilStateProvider {
    fn basic_account(&self, address: &Address) -> ProviderResult<Option<Account>> {
        let Some(account_override) = self.state.read().account(*address) else {
            return self.inner.basic_account(address);
        };

        let mut account = self.inner.basic_account(address)?.unwrap_or_default();
        if let Some(balance) = account_override.balance() {
            account.balance = balance;
        }
        if let Some(nonce) = account_override.nonce() {
            account.nonce = nonce;
        }
        if let Some(code_hash) = account_override.code_hash() {
            account.bytecode_hash = Some(code_hash);
        }
        Ok(Some(account))
    }
}

impl StateRootProvider for AnvilStateProvider {
    fn state_root(&self, hashed_state: HashedPostState) -> ProviderResult<B256> {
        self.inner.state_root(hashed_state)
    }

    fn state_root_from_nodes(&self, input: TrieInput) -> ProviderResult<B256> {
        self.inner.state_root_from_nodes(input)
    }

    fn state_root_with_updates(
        &self,
        hashed_state: HashedPostState,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        self.inner.state_root_with_updates(hashed_state)
    }

    fn state_root_from_nodes_with_updates(
        &self,
        input: TrieInput,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        self.inner.state_root_from_nodes_with_updates(input)
    }
}

impl StorageRootProvider for AnvilStateProvider {
    fn storage_root(
        &self,
        address: Address,
        hashed_storage: HashedStorage,
    ) -> ProviderResult<B256> {
        self.inner.storage_root(address, hashed_storage)
    }

    fn storage_proof(
        &self,
        address: Address,
        slot: B256,
        hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageProof> {
        self.inner.storage_proof(address, slot, hashed_storage)
    }

    fn storage_multiproof(
        &self,
        address: Address,
        slots: &[B256],
        hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageMultiProof> {
        self.inner
            .storage_multiproof(address, slots, hashed_storage)
    }
}

impl StateProofProvider for AnvilStateProvider {
    fn proof(
        &self,
        input: TrieInput,
        address: Address,
        slots: &[B256],
    ) -> ProviderResult<AccountProof> {
        self.inner.proof(input, address, slots)
    }

    fn multiproof(
        &self,
        input: TrieInput,
        targets: MultiProofTargets,
    ) -> ProviderResult<MultiProof> {
        self.inner.multiproof(input, targets)
    }

    fn witness(
        &self,
        input: TrieInput,
        target: HashedPostState,
        mode: ExecutionWitnessMode,
    ) -> ProviderResult<Vec<alloy_primitives::Bytes>> {
        self.inner.witness(input, target, mode)
    }
}

impl BlockHashReader for AnvilStateProvider {
    fn block_hash(&self, number: u64) -> ProviderResult<Option<B256>> {
        self.inner.block_hash(number)
    }

    fn convert_block_hash(
        &self,
        hash_or_number: alloy_rpc_types_eth::BlockHashOrNumber,
    ) -> ProviderResult<Option<B256>> {
        self.inner.convert_block_hash(hash_or_number)
    }

    fn canonical_hashes_range(
        &self,
        start: alloy_primitives::BlockNumber,
        end: alloy_primitives::BlockNumber,
    ) -> ProviderResult<Vec<B256>> {
        self.inner.canonical_hashes_range(start, end)
    }
}

impl HashedPostStateProvider for AnvilStateProvider {
    fn hashed_post_state(&self, bundle_state: &BundleState) -> HashedPostState {
        self.inner.hashed_post_state(bundle_state)
    }
}

impl StateProvider for AnvilStateProvider {
    fn storage(
        &self,
        account: Address,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let Some(value) = self.state.read().storage(account, storage_key) {
            return Ok(Some(value));
        }

        self.inner.storage(account, storage_key)
    }
}

impl BytecodeReader for AnvilStateProvider {
    fn bytecode_by_hash(&self, code_hash: &B256) -> ProviderResult<Option<Bytecode>> {
        if let Some(bytecode) = self.state.read().bytecode_by_hash(code_hash) {
            return Ok(Some(bytecode));
        }

        self.inner.bytecode_by_hash(code_hash)
    }
}
