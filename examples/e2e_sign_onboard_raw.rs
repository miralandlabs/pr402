//! Sign the **exact UTF-8 bytes** of `POST /onboard` `message` (HMAC body + line).
//! Do **not** use `solana sign-offchain-message` — that adds the Solana off-chain framing;
//! pr402 [`pr402::onboard_auth::verify_onboard_submission`] verifies `ed25519(message)`.

use solana_keypair::{EncodableKey, Keypair};
use solana_signer::Signer;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let keypair_path = args
        .next()
        .ok_or("usage: e2e_sign_onboard_raw <keypair.json> <message.txt>")?;
    let message_path = args
        .next()
        .ok_or("usage: e2e_sign_onboard_raw <keypair.json> <message.txt>")?;
    let kp = Keypair::read_from_file(&keypair_path)?;
    let msg = std::fs::read_to_string(&message_path)?;
    let sig = kp.sign_message(msg.as_bytes());
    println!("{sig}");
    Ok(())
}
