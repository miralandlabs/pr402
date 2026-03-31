//! Decode `bincode` wire bytes as a [`VersionedTransaction`].
//!
//! The SLA-escrow CLI (`send_and_confirm`) serializes legacy [`solana_transaction::Transaction`].
//! `build-exact-payment-tx` returns a legacy-shell [`VersionedTransaction`]. Both must deserialize.

use solana_transaction::{versioned::VersionedTransaction, Transaction};

/// Deserialize `bincode` bytes produced by either legacy `Transaction` or `VersionedTransaction`.
pub fn decode_versioned_transaction_from_bincode(
    bytes: &[u8],
) -> Result<VersionedTransaction, String> {
    match bincode::deserialize::<VersionedTransaction>(bytes) {
        Ok(v) => Ok(v),
        Err(_) => {
            let legacy: Transaction = bincode::deserialize(bytes).map_err(|e| {
                format!("transaction bincode is neither versioned nor legacy: {}", e)
            })?;
            Ok(VersionedTransaction::from(legacy))
        }
    }
}
