use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_primitives::{keccak256, Address, Bytes, StorageKey, StorageValue, B256, U256};
use parking_lot::RwLock;
use reth_primitives_traits::Bytecode;
use std::{collections::HashMap, sync::Arc};

/// Shared Anvil dev-state handle.
pub type SharedAnvilState = Arc<RwLock<AnvilState>>;

/// Local Anvil-owned mutable state.
///
/// This is intentionally kept above Reth storage internals so Anvil semantics do not depend on
/// table layout details like storage v1/v2.
#[derive(Debug, Default)]
pub struct AnvilState {
    accounts: HashMap<Address, AccountOverride>,
    bytecodes: HashMap<B256, Bytecode>,
}

#[derive(Debug, Default, Clone)]
pub struct AccountOverride {
    balance: Option<U256>,
    nonce: Option<u64>,
    code: Option<CodeOverride>,
    storage: HashMap<StorageKey, StorageValue>,
}

#[derive(Debug, Clone)]
struct CodeOverride {
    hash: B256,
}

impl AccountOverride {
    pub fn balance(&self) -> Option<U256> {
        self.balance
    }

    pub fn nonce(&self) -> Option<u64> {
        self.nonce
    }

    pub fn code_hash(&self) -> Option<B256> {
        self.code.as_ref().map(|code| code.hash)
    }

    pub fn storage(&self) -> &HashMap<StorageKey, StorageValue> {
        &self.storage
    }
}

impl AnvilState {
    /// Create a new empty state handle.
    pub fn shared() -> SharedAnvilState {
        Arc::new(RwLock::new(Self::default()))
    }

    /// Override the balance for the given account.
    pub fn set_balance(&mut self, address: Address, balance: U256) {
        self.accounts.entry(address).or_default().balance = Some(balance);
    }

    /// Override the nonce for the given account.
    pub fn set_nonce(&mut self, address: Address, nonce: u64) {
        self.accounts.entry(address).or_default().nonce = Some(nonce);
    }

    /// Override the code for the given account.
    pub fn set_code(&mut self, address: Address, code: Bytes) {
        let override_code = if code.is_empty() {
            CodeOverride { hash: KECCAK_EMPTY }
        } else {
            let hash = keccak256(code.as_ref());
            let bytecode = Bytecode::new_raw(code);
            self.bytecodes.insert(hash, bytecode.clone());
            CodeOverride { hash }
        };

        self.accounts.entry(address).or_default().code = Some(override_code);
    }

    /// Override a single storage slot for the given account.
    pub fn set_storage_at(&mut self, address: Address, slot: StorageKey, value: StorageValue) {
        self.accounts
            .entry(address)
            .or_default()
            .storage
            .insert(slot, value);
    }

    /// Returns the local account state for the given address, if any.
    pub(crate) fn account(&self, address: Address) -> Option<AccountOverride> {
        self.accounts.get(&address).cloned()
    }

    /// Returns all local account state.
    pub(crate) fn accounts(&self) -> Vec<(Address, AccountOverride)> {
        self.accounts
            .iter()
            .map(|(address, account)| (*address, account.clone()))
            .collect()
    }

    /// Returns locally overridden bytecode for the given code hash, if any.
    pub(crate) fn bytecode_by_hash(&self, code_hash: &B256) -> Option<Bytecode> {
        self.bytecodes.get(code_hash).cloned()
    }

    /// Returns locally overridden storage for the given account and slot, if any.
    pub(crate) fn storage(&self, address: Address, slot: StorageKey) -> Option<StorageValue> {
        self.accounts
            .get(&address)
            .and_then(|account| account.storage.get(&slot).copied())
    }
}
