use crate::state::AnvilState;
use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_primitives::{Address, Bytes, B256, U256};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use reth_storage_api::{DumpedAccount, DumpedState};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct SerializableState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<serde_json::Value>,
    #[serde(default)]
    pub accounts: BTreeMap<Address, SerializableAccountRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_block_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transactions: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub historical_states: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct SerializableAccountRecord {
    #[serde(default)]
    pub nonce: u64,
    #[serde(default)]
    pub balance: U256,
    #[serde(default)]
    pub code: Bytes,
    #[serde(default)]
    pub storage: BTreeMap<B256, B256>,
}

impl SerializableState {
    pub(crate) fn from_dump(dump: DumpedState, best_block_number: Option<u64>) -> Self {
        let accounts = dump
            .accounts
            .into_iter()
            .map(|account| (account.address, SerializableAccountRecord::from(account)))
            .collect();

        Self {
            accounts,
            best_block_number,
            ..Default::default()
        }
    }

    pub(crate) fn merge_anvil_state(&mut self, anvil_state: &AnvilState) {
        for (address, account) in anvil_state.accounts() {
            let record = self.accounts.entry(address).or_default();

            if let Some(balance) = account.balance() {
                record.balance = balance;
            }
            if let Some(nonce) = account.nonce() {
                record.nonce = nonce;
            }
            if let Some(code_hash) = account.code_hash() {
                record.code = if code_hash == KECCAK_EMPTY {
                    Bytes::new()
                } else {
                    anvil_state
                        .bytecode_by_hash(&code_hash)
                        .map(|bytecode| bytecode.original_bytes())
                        .unwrap_or_default()
                };
            }

            for (slot, value) in account.storage() {
                record
                    .storage
                    .insert(*slot, B256::from(value.to_be_bytes()));
            }
        }
    }

    pub(crate) fn encode_gzipped(&self) -> Result<Bytes, String> {
        let json = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&json)
            .map_err(|error| error.to_string())?;
        encoder
            .finish()
            .map(Bytes::from)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn decode(buf: &Bytes) -> Result<Self, String> {
        let mut decoder = GzDecoder::new(buf.as_ref());
        if decoder.header().is_some() {
            let mut decoded = Vec::new();
            decoder
                .read_to_end(&mut decoded)
                .map_err(|error| error.to_string())?;
            return serde_json::from_slice(&decoded).map_err(|error| error.to_string());
        }

        serde_json::from_slice(buf.as_ref()).map_err(|error| error.to_string())
    }
}

impl From<DumpedAccount> for SerializableAccountRecord {
    fn from(account: DumpedAccount) -> Self {
        Self {
            nonce: account.nonce,
            balance: account.balance,
            code: account.code.unwrap_or_default(),
            storage: account.storage,
        }
    }
}
