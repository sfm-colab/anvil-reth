use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_network::TxSigner;
use alloy_primitives::{Address, Signature, B256};
use alloy_signer::Result as SignerResult;
use jsonrpsee::core::async_trait;
use reth_rpc_eth_api::helpers::EthSigner;
use reth_rpc_eth_types::SignError;
use reth_rpc_traits::SignableTxRequest;
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

/// Shared impersonation state, accessible from both the pool validator and
/// the RPC layer.
#[derive(Debug, Clone, Default)]
pub struct ImpersonationState {
    inner: Arc<RwLock<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    accounts: HashSet<Address>,
    auto_impersonate: bool,
    /// Maps impersonated tx hashes to the intended sender address so the
    /// engine can attribute them correctly during payload execution.
    tx_senders: HashMap<B256, Address>,
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

    pub fn impersonated_accounts(&self) -> Vec<Address> {
        self.inner
            .read()
            .unwrap()
            .accounts
            .iter()
            .copied()
            .collect()
    }

    pub fn remember_tx_sender(&self, hash: B256, sender: Address) {
        self.inner.write().unwrap().tx_senders.insert(hash, sender);
    }

    pub fn forget_tx_sender(&self, hash: &B256) {
        self.inner.write().unwrap().tx_senders.remove(hash);
    }

    pub fn forget_tx_senders(&self, hashes: impl IntoIterator<Item = B256>) {
        let mut inner = self.inner.write().unwrap();
        for hash in hashes {
            inner.tx_senders.remove(&hash);
        }
    }

    pub fn tx_sender(&self, hash: &B256) -> Option<Address> {
        self.inner.read().unwrap().tx_senders.get(hash).copied()
    }
}

/// Signs transaction requests with a placeholder signature while preserving the
/// requested sender through the surrounding `Recovered<T>` wrapper.
#[derive(Debug, Clone, Copy)]
struct ImpersonatedTxSigner {
    address: Address,
}

#[async_trait]
impl TxSigner<Signature> for ImpersonatedTxSigner {
    fn address(&self) -> Address {
        self.address
    }

    async fn sign_transaction(
        &self,
        _tx: &mut dyn SignableTransaction<Signature>,
    ) -> SignerResult<Signature> {
        Ok(Signature::new(
            Default::default(),
            Default::default(),
            false,
        ))
    }
}

/// Dynamic signer that becomes available for any explicitly or automatically
/// impersonated account.
#[derive(Debug)]
pub struct ImpersonatedSigner<T, TxReq> {
    state: ImpersonationState,
    marker: PhantomData<fn() -> (T, TxReq)>,
}

impl<T, TxReq> ImpersonatedSigner<T, TxReq> {
    pub fn new(state: ImpersonationState) -> Self {
        Self {
            state,
            marker: PhantomData,
        }
    }
}

impl<T, TxReq> Clone for ImpersonatedSigner<T, TxReq> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            marker: PhantomData,
        }
    }
}

#[async_trait]
impl<T, TxReq> EthSigner<T, TxReq> for ImpersonatedSigner<T, TxReq>
where
    T: Send + Sync + 'static,
    TxReq: SignableTxRequest<T> + Send + Sync + 'static,
{
    fn accounts(&self) -> Vec<Address> {
        self.state.impersonated_accounts()
    }

    fn is_signer_for(&self, address: &Address) -> bool {
        self.state.is_impersonated(address)
    }

    async fn sign(&self, _address: Address, _message: &[u8]) -> Result<Signature, SignError> {
        Err(SignError::CouldNotSign)
    }

    async fn sign_transaction(&self, request: TxReq, address: &Address) -> Result<T, SignError> {
        request
            .try_build_and_sign(ImpersonatedTxSigner { address: *address })
            .await
            .map_err(|_| SignError::InvalidTransactionRequest)
    }

    fn sign_typed_data(
        &self,
        _address: Address,
        _payload: &TypedData,
    ) -> Result<Signature, SignError> {
        Err(SignError::CouldNotSign)
    }
}
