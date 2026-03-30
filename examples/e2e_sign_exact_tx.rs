//! Devnet E2E helper: sign an **unsigned** legacy-shell `VersionedTransaction` from
//! `POST /api/v1/facilitator/build-exact-payment-tx` so it can be pasted into `/verify`.
//!
//! ```text
//! curl -s .../build-exact-payment-tx -d @body.json | jq -r .transaction \
//!   | cargo run --example e2e_sign_exact_tx -- <payer.json> <recentBlockhash>
//! ```

use base64::{engine::general_purpose::STANDARD, Engine};
use solana_hash::Hash;
use solana_keypair::{EncodableKey, Keypair};
use solana_transaction::versioned::VersionedTransaction;
use std::io::Read;
use std::str::FromStr;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let keypair_path = args.next().ok_or(
        "usage: cargo run --example e2e_sign_exact_tx -- <payer-keypair.json> <recentBlockhash-b58>\n\
         stdin: base64(bincode VersionedTransaction) from build-exact response field `transaction`",
    )?;
    let recent_bh = args
        .next()
        .ok_or("missing recent blockhash (from build-exact JSON)")?;

    let mut stdin = String::new();
    std::io::stdin().read_to_string(&mut stdin)?;
    let tx_b64 = stdin.trim();
    let bytes = STANDARD.decode(tx_b64)?;
    let vtx = pr402::util::decode_versioned_transaction_from_bincode(&bytes)
        .map_err(|e| anyhow::anyhow!("decode vtx: {}", e))?;

    let hash = Hash::from_str(&recent_bh)?;
    let kp = Keypair::read_from_file(&keypair_path)?;

    let signed = match vtx.into_legacy_transaction() {
        Some(mut legacy) => {
            legacy.try_partial_sign(&[&kp], hash)?;
            VersionedTransaction::from(legacy)
        }
        None => return Err("only legacy-shell transactions are supported by this example".into()),
    };

    let out = bincode::serialize(&signed)?;
    println!("{}", STANDARD.encode(out));
    Ok(())
}
