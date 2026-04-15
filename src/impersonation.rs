use alloy_primitives::Address;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// Shared impersonation state, accessible from both the pool validator and
/// the anvil_* RPC handlers.
#[derive(Debug, Clone, Default)]
pub struct ImpersonationState {
    inner: Arc<RwLock<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    accounts: HashSet<Address>,
    auto_impersonate: bool,
}

impl ImpersonationState {
    pub fn impersonate(&self, address: Address) {
        self.inner.write().unwrap().accounts.insert(address);
    }

    pub fn stop_impersonating(&self, address: Address) {
        self.inner.write().unwrap().accounts.remove(&address);
    }

    pub fn set_auto_impersonate(&self, enabled: bool) {
        self.inner.write().unwrap().auto_impersonate = enabled;
    }

    pub fn is_impersonated(&self, address: &Address) -> bool {
        let inner = self.inner.read().unwrap();
        inner.auto_impersonate || inner.accounts.contains(address)
    }
}
