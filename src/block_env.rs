use alloy_primitives::Address;
use parking_lot::RwLock;
use std::sync::Arc;

/// Restorable block-environment override state.
#[derive(Clone, Debug, Default)]
pub struct BlockEnvSnapshot {
    gas_limit: Option<u64>,
    coinbase: Option<Address>,
    next_base_fee: Option<u64>,
}

/// Shared block-environment overrides for block gas limit, coinbase, and next-block base fee.
///
/// `gas_limit` and `coinbase` are persistent — they apply to every subsequent block
/// until changed again. `next_base_fee` is consumed once when the next block is built.
#[derive(Clone, Debug, Default)]
pub struct BlockEnvOverrides {
    gas_limit: Arc<RwLock<Option<u64>>>,
    coinbase: Arc<RwLock<Option<Address>>>,
    next_base_fee: Arc<RwLock<Option<u64>>>,
}

impl BlockEnvOverrides {
    /// Sets a persistent gas limit override for all future blocks.
    pub fn set_gas_limit(&self, limit: u64) {
        *self.gas_limit.write() = Some(limit);
    }

    /// Returns the gas limit override, if set.
    pub fn gas_limit(&self) -> Option<u64> {
        *self.gas_limit.read()
    }

    /// Sets a persistent coinbase override for all future blocks.
    pub fn set_coinbase(&self, address: Address) {
        *self.coinbase.write() = Some(address);
    }

    /// Returns the coinbase override, if set.
    pub fn coinbase(&self) -> Option<Address> {
        *self.coinbase.read()
    }

    /// Sets the base fee for the next block only. Consumed on use.
    pub fn set_next_base_fee(&self, fee: u64) {
        *self.next_base_fee.write() = Some(fee);
    }

    /// Takes the next-block base fee override, consuming it.
    pub fn take_next_base_fee(&self) -> Option<u64> {
        self.next_base_fee.write().take()
    }

    /// Captures the current block environment overrides.
    pub fn snapshot(&self) -> BlockEnvSnapshot {
        BlockEnvSnapshot {
            gas_limit: *self.gas_limit.read(),
            coinbase: *self.coinbase.read(),
            next_base_fee: *self.next_base_fee.read(),
        }
    }

    /// Restores block environment overrides from a snapshot.
    pub fn restore(&self, snapshot: BlockEnvSnapshot) {
        *self.gas_limit.write() = snapshot.gas_limit;
        *self.coinbase.write() = snapshot.coinbase;
        *self.next_base_fee.write() = snapshot.next_base_fee;
    }
}
