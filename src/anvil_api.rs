use alloy_primitives::Address;
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::proc_macros::rpc;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// anvil_* RPC namespace.
///
/// Mirrors the trait defined in `reth-rpc-api`, which does not publicly export
/// `AnvilApiServer` in its `servers` module.
#[rpc(server, namespace = "anvil")]
pub trait AnvilApi {
    #[method(name = "impersonateAccount")]
    async fn anvil_impersonate_account(&self, address: Address) -> RpcResult<()>;

    #[method(name = "stopImpersonatingAccount")]
    async fn anvil_stop_impersonating_account(&self, address: Address) -> RpcResult<()>;

    #[method(name = "autoImpersonateAccount")]
    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> RpcResult<()>;

    #[method(name = "getAutomine")]
    async fn anvil_get_automine(&self) -> RpcResult<bool>;

    #[method(name = "setLoggingEnabled")]
    async fn anvil_set_logging_enabled(&self, enabled: bool) -> RpcResult<()>;
}

/// Shared state for impersonated accounts.
#[derive(Debug, Default)]
struct AnvilState {
    impersonated: HashSet<Address>,
    auto_impersonate: bool,
}

/// Implementation of the `anvil_*` RPC namespace.
#[derive(Debug, Clone)]
pub struct AnvilRpc {
    state: Arc<RwLock<AnvilState>>,
}

impl AnvilRpc {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(AnvilState::default())),
        }
    }
}

#[async_trait]
impl AnvilApiServer for AnvilRpc {
    async fn anvil_impersonate_account(&self, address: Address) -> RpcResult<()> {
        self.state.write().unwrap().impersonated.insert(address);
        Ok(())
    }

    async fn anvil_stop_impersonating_account(&self, address: Address) -> RpcResult<()> {
        self.state.write().unwrap().impersonated.remove(&address);
        Ok(())
    }

    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> RpcResult<()> {
        self.state.write().unwrap().auto_impersonate = enabled;
        Ok(())
    }

    async fn anvil_get_automine(&self) -> RpcResult<bool> {
        Ok(true)
    }

    async fn anvil_set_logging_enabled(&self, _enabled: bool) -> RpcResult<()> {
        Ok(())
    }
}
