use alloy_primitives::{Address, U256};
use parking_lot::RwLock;
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
}

#[derive(Debug, Default, Clone, Copy)]
struct AccountOverride {
    balance: Option<U256>,
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

    /// Returns the overridden balance for the given account, if any.
    pub fn balance_override(&self, address: Address) -> Option<U256> {
        self.accounts
            .get(&address)
            .and_then(|account| account.balance)
    }
}
