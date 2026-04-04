//! Decode `bincode` wire bytes as a [`VersionedTransaction`].
//!
//! The SLA-escrow CLI (`send_and_confirm`) serializes legacy [`solana_transaction::Transaction`].
//! `build-exact-payment-tx` returns a legacy-shell [`VersionedTransaction`]. Both must deserialize.

use solana_transaction::{versioned::VersionedTransaction, Transaction};

/// Reject v0 messages that load accounts via on-chain address lookup tables.
///
/// Verification walks instructions with [`solana_message::VersionedMessage::static_account_keys`]
/// only; ALT-loaded addresses are not supported yet.
pub fn reject_versioned_tx_with_address_lookup_tables(
    tx: &VersionedTransaction,
) -> Result<(), String> {
    if let Some(lookups) = tx.message.address_table_lookups() {
        if !lookups.is_empty() {
            return Err(
                "versioned transactions with address lookup tables are not supported".into(),
            );
        }
    }
    Ok(())
}

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
