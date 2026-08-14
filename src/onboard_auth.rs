//! Wallet-signed onboarding: HMAC binds challenge fields to the deployment; ed25519 proves key control.
//!
//! The facilitator only needs `PR402_ONBOARD_HMAC_SECRET` (deployment secret, not an RP API key).
//! Resource providers prove control with their normal Solana keypair signature.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

const DOMAIN_V1: &str = "pr402 facilitator onboard v1";
const DOMAIN_V2: &str = "pr402 facilitator onboard v2";
const HMAC_LINE_PREFIX: &str = "hmac_sha256_hex: ";

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn preimage_for_hmac(wallet: &str, issued_unix: u64, expires_unix: u64, nonce_hex: &str) -> String {
    format!(
        "{DOMAIN_V1}\nwallet: {wallet}\nissued_unix: {issued_unix}\nexpires_unix: {expires_unix}\nnonce: {nonce_hex}\n"
    )
}

fn preimage_for_hmac_v2(
    wallet: &str,
    action: &str,
    issued_unix: u64,
    expires_unix: u64,
    nonce_hex: &str,
) -> String {
    format!(
        "{DOMAIN_V2}\nwallet: {wallet}\naction: {action}\nissued_unix: {issued_unix}\nexpires_unix: {expires_unix}\nnonce: {nonce_hex}\n"
    )
}

fn valid_action(action: &str) -> bool {
    !action.is_empty()
        && action.len() <= 64
        && action
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, ':' | '-' | '_'))
}

fn action_authorizes(actual: &str, expected: &str) -> bool {
    actual == expected
        || (actual == "seller:any"
            && matches!(
                expected,
                "seller:register" | "seller:retire" | "seller:payments" | "resource:list"
            ))
        || (actual == "resource:any"
            && matches!(
                expected,
                "resource:register" | "resource:retire" | "resource:probe"
            ))
}

fn compute_hmac_hex(secret: &[u8], preimage: &str) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| "invalid HMAC key length")?;
    mac.update(preimage.as_bytes());
    let out = mac.finalize().into_bytes();
    Ok(hex_lower(&out))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        write!(&mut s, "{b:02x}").ok();
    }
    s
}

fn parse_nonce_hex(s: &str) -> bool {
    s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Decode an ed25519 signature given as **base58** (canonical Solana encoding) or
/// **base64** (what browser wallet adapters frequently return). Signatures are
/// always 64 bytes, which is how we disambiguate: we try base58 first (the
/// canonical form), and fall back to base64 when that fails. Rejects anything
/// that doesn't decode to exactly 64 bytes.
///
/// Why both: `@solana/wallet-adapter-react` `useWallet().signMessage` returns a
/// `Uint8Array` which most integrators then base64-encode over the wire; the
/// facilitator historically only accepted base58, forcing clients to carry a
/// base58 encoder just for this one call. Accepting both removes that
/// side-quest from agents and browser pages.
fn decode_signature_flexible(encoded: &str) -> Result<Signature, String> {
    let raw = encoded.trim();
    if raw.is_empty() {
        return Err("empty signature".into());
    }

    if let Ok(sig) = Signature::from_str(raw) {
        return Ok(sig);
    }

    use base64::{engine::general_purpose::STANDARD as B64_STD, Engine};
    let bytes = B64_STD
        .decode(raw)
        .map_err(|_| "invalid signature encoding (expected base58 or base64)")?;
    if bytes.len() != 64 {
        return Err(format!(
            "invalid signature length: expected 64 bytes, got {}",
            bytes.len()
        ));
    }
    let arr: [u8; 64] = bytes
        .try_into()
        .map_err(|_| "invalid signature length after base64 decode")?;
    Ok(Signature::from(arr))
}

/// Build the exact UTF-8 string the wallet must sign for POST `/api/v1/facilitator/sellers/{wallet}/register`.
pub fn build_signed_onboard_message(
    hmac_secret: &[u8],
    wallet_b58: &str,
    ttl_sec: u64,
) -> Result<(String, u64), String> {
    if ttl_sec == 0 || ttl_sec > 3600 {
        return Err("ttl_sec must be 1..=3600".into());
    }
    Pubkey::from_str(wallet_b58).map_err(|_| "invalid wallet pubkey")?;

    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| "RNG failure")?;
    let nonce_hex = hex_lower(&nonce);
    let issued = now_unix();
    let expires = issued.saturating_add(ttl_sec);
    let preimage = preimage_for_hmac(wallet_b58, issued, expires, &nonce_hex);
    let hmac_hex = compute_hmac_hex(hmac_secret, &preimage)?;
    let message = format!("{preimage}{HMAC_LINE_PREFIX}{hmac_hex}");
    Ok((message, expires))
}

/// Build a v2 challenge bound to one server-side action.
pub fn build_signed_onboard_message_for_action(
    hmac_secret: &[u8],
    wallet_b58: &str,
    action: &str,
    ttl_sec: u64,
) -> Result<(String, u64), String> {
    if ttl_sec == 0 || ttl_sec > 3600 {
        return Err("ttl_sec must be 1..=3600".into());
    }
    Pubkey::from_str(wallet_b58).map_err(|_| "invalid wallet pubkey")?;
    if !valid_action(action) {
        return Err("invalid challenge action".into());
    }

    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| "RNG failure")?;
    let nonce_hex = hex_lower(&nonce);
    let issued = now_unix();
    let expires = issued.saturating_add(ttl_sec);
    let preimage = preimage_for_hmac_v2(wallet_b58, action, issued, expires, &nonce_hex);
    let hmac_hex = compute_hmac_hex(hmac_secret, &preimage)?;
    let message = format!("{preimage}{HMAC_LINE_PREFIX}{hmac_hex}");
    Ok((message, expires))
}

fn legacy_challenges_allowed() -> bool {
    std::env::var("PR402_ALLOW_LEGACY_ONBOARD_CHALLENGES")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

/// Verify HMAC + expiry + wallet line, then ed25519(signature, message, pubkey).
///
/// Accepts either **base58** (canonical Solana signature encoding — what
/// `Signature::from_str` parses) or **base64** (what `@solana/wallet-adapter`
/// and many JS wallets return natively from `signMessage`). Auto-detects on
/// decoded length: an ed25519 signature is always 64 bytes.
pub fn verify_onboard_submission(
    hmac_secret: &[u8],
    wallet_b58: &str,
    message: &str,
    signature_encoded: &str,
) -> Result<(), String> {
    verify_onboard_submission_inner(hmac_secret, wallet_b58, message, signature_encoded, None)
}

/// Verify a v2 challenge and require its signed action to match this handler.
/// Legacy v1 messages are rejected unless the deployment explicitly enables the
/// temporary `PR402_ALLOW_LEGACY_ONBOARD_CHALLENGES` compatibility switch.
pub fn verify_onboard_submission_for_action(
    hmac_secret: &[u8],
    wallet_b58: &str,
    message: &str,
    signature_encoded: &str,
    expected_action: &str,
) -> Result<(), String> {
    verify_onboard_submission_inner(
        hmac_secret,
        wallet_b58,
        message,
        signature_encoded,
        Some(expected_action),
    )
}

/// Verify an action-bound challenge and atomically mark it used when persistence is enabled.
pub async fn verify_and_consume_onboard_submission_for_action(
    db: Option<&crate::db::Pr402Db>,
    hmac_secret: &[u8],
    wallet_b58: &str,
    message: &str,
    signature_encoded: &str,
    expected_action: &str,
) -> Result<(), String> {
    verify_onboard_submission_for_action(
        hmac_secret,
        wallet_b58,
        message,
        signature_encoded,
        expected_action,
    )?;
    if let Some(db) = db {
        let challenge_sha256 = hex_lower(&Sha256::digest(message.as_bytes()));
        let inserted = db
            .consume_onboard_challenge(&challenge_sha256, wallet_b58)
            .await
            .map_err(|error| format!("challenge replay check failed: {error}"))?;
        if !inserted {
            return Err("challenge already used; request and sign a fresh challenge".into());
        }
    }
    Ok(())
}

fn verify_onboard_submission_inner(
    hmac_secret: &[u8],
    wallet_b58: &str,
    message: &str,
    signature_encoded: &str,
    expected_action: Option<&str>,
) -> Result<(), String> {
    let pk = Pubkey::from_str(wallet_b58).map_err(|_| "invalid wallet pubkey")?;
    let sig = decode_signature_flexible(signature_encoded)?;

    let Some(idx) = message.rfind(HMAC_LINE_PREFIX) else {
        return Err("missing HMAC line".into());
    };
    let body = &message[..idx];
    let hmac_line = &message[idx..];
    let Some(hmac_hex) = hmac_line.strip_prefix(HMAC_LINE_PREFIX) else {
        return Err("invalid HMAC line".into());
    };
    if hmac_hex.len() != 64 || !hmac_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid HMAC hex".into());
    }

    let expected_mac = compute_hmac_hex(hmac_secret, body)?;
    if expected_mac.len() != hmac_hex.len()
        || !bool::from(expected_mac.as_bytes().ct_eq(hmac_hex.as_bytes()))
    {
        return Err("HMAC mismatch".into());
    }

    let lines: Vec<&str> = body.lines().collect();
    let (wallet_index, issued_index, expires_index, nonce_index) = match lines.first().copied() {
        Some(DOMAIN_V2) if lines.len() == 6 => {
            let action = lines[2]
                .strip_prefix("action: ")
                .ok_or_else(|| "invalid action line".to_string())?;
            if !valid_action(action) {
                return Err("invalid challenge action".into());
            }
            if expected_action.is_some_and(|expected| !action_authorizes(action, expected)) {
                return Err(format!(
                    "challenge action mismatch: expected {}, got {action}",
                    expected_action.unwrap_or_default()
                ));
            }
            (1, 3, 4, 5)
        }
        Some(DOMAIN_V1) if lines.len() == 5 => {
            if expected_action.is_some() && !legacy_challenges_allowed() {
                return Err(
                    "legacy unscoped challenge rejected; request a fresh action-bound challenge"
                        .into(),
                );
            }
            (1, 2, 3, 4)
        }
        _ => return Err("invalid challenge domain or line count".into()),
    };
    let Some(rest) = lines[wallet_index].strip_prefix("wallet: ") else {
        return Err("invalid wallet line".into());
    };
    if rest != wallet_b58 {
        return Err("wallet mismatch".into());
    }

    let issued = lines[issued_index]
        .strip_prefix("issued_unix: ")
        .ok_or_else(|| "issued_unix".to_string())?
        .parse::<u64>()
        .map_err(|_| "issued_unix".to_string())?;
    let expires = lines[expires_index]
        .strip_prefix("expires_unix: ")
        .ok_or_else(|| "expires_unix".to_string())?
        .parse::<u64>()
        .map_err(|_| "expires_unix".to_string())?;
    let nonce_line = lines[nonce_index]
        .strip_prefix("nonce: ")
        .ok_or_else(|| "nonce".to_string())?;
    if !parse_nonce_hex(nonce_line) {
        return Err("invalid nonce".into());
    }

    let t = now_unix();
    if t < issued {
        return Err("issued in the future".into());
    }
    if t > expires {
        return Err("challenge expired".into());
    }

    if !sig.verify(pk.as_ref(), message.as_bytes()) {
        return Err("invalid signature".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_signer::Signer;

    #[test]
    fn round_trip_ok() {
        let secret = b"test-secret-at-least-32-bytes-long!!";
        let kp = solana_keypair::Keypair::new();
        let wallet = kp.pubkey().to_string();
        let (msg, _exp) = build_signed_onboard_message(secret, &wallet, 600).unwrap();
        let sig = kp.sign_message(msg.as_bytes());
        verify_onboard_submission(secret, &wallet, &msg, &sig.to_string()).unwrap();
    }

    #[test]
    fn wrong_wallet_fails() {
        let secret = b"test-secret-at-least-32-bytes-long!!";
        let kp = solana_keypair::Keypair::new();
        let wallet = kp.pubkey().to_string();
        let (msg, _exp) = build_signed_onboard_message(secret, &wallet, 600).unwrap();
        let sig = kp.sign_message(msg.as_bytes());
        let wrong = solana_keypair::Keypair::new().pubkey().to_string();
        assert!(verify_onboard_submission(secret, &wrong, &msg, &sig.to_string()).is_err());
    }

    #[test]
    fn action_bound_challenge_only_authorizes_its_handler() {
        let secret = b"test-secret-at-least-32-bytes-long!!";
        let kp = solana_keypair::Keypair::new();
        let wallet = kp.pubkey().to_string();
        let (msg, _) =
            build_signed_onboard_message_for_action(secret, &wallet, "seller:register", 600)
                .unwrap();
        let sig = kp.sign_message(msg.as_bytes());

        verify_onboard_submission_for_action(
            secret,
            &wallet,
            &msg,
            &sig.to_string(),
            "seller:register",
        )
        .unwrap();
        assert!(verify_onboard_submission_for_action(
            secret,
            &wallet,
            &msg,
            &sig.to_string(),
            "seller:retire",
        )
        .is_err());
    }

    #[test]
    fn compatibility_scopes_authorize_only_their_legacy_endpoint_family() {
        let secret = b"test-secret-at-least-32-bytes-long!!";
        let kp = solana_keypair::Keypair::new();
        let wallet = kp.pubkey().to_string();

        let (seller_msg, _) =
            build_signed_onboard_message_for_action(secret, &wallet, "seller:any", 600).unwrap();
        let seller_sig = kp.sign_message(seller_msg.as_bytes()).to_string();
        verify_onboard_submission_for_action(
            secret,
            &wallet,
            &seller_msg,
            &seller_sig,
            "seller:retire",
        )
        .unwrap();
        assert!(verify_onboard_submission_for_action(
            secret,
            &wallet,
            &seller_msg,
            &seller_sig,
            "resource:probe",
        )
        .is_err());

        let (resource_msg, _) =
            build_signed_onboard_message_for_action(secret, &wallet, "resource:any", 600).unwrap();
        let resource_sig = kp.sign_message(resource_msg.as_bytes()).to_string();
        verify_onboard_submission_for_action(
            secret,
            &wallet,
            &resource_msg,
            &resource_sig,
            "resource:probe",
        )
        .unwrap();
        assert!(verify_onboard_submission_for_action(
            secret,
            &wallet,
            &resource_msg,
            &resource_sig,
            "seller:retire",
        )
        .is_err());
    }

    #[test]
    fn accepts_base64_signature() {
        use base64::{engine::general_purpose::STANDARD as B64_STD, Engine};
        let secret = b"test-secret-at-least-32-bytes-long!!";
        let kp = solana_keypair::Keypair::new();
        let wallet = kp.pubkey().to_string();
        let (msg, _exp) = build_signed_onboard_message(secret, &wallet, 600).unwrap();
        let sig = kp.sign_message(msg.as_bytes());
        // Encode the 64-byte signature as base64 instead of the canonical base58.
        // The server must accept both (browser wallets return base64 natively).
        let sig_b64 = B64_STD.encode(<[u8; 64]>::from(sig));
        verify_onboard_submission(secret, &wallet, &msg, &sig_b64).unwrap();
    }

    #[test]
    fn rejects_wrong_length_base64() {
        let secret = b"test-secret-at-least-32-bytes-long!!";
        let kp = solana_keypair::Keypair::new();
        let wallet = kp.pubkey().to_string();
        let (msg, _exp) = build_signed_onboard_message(secret, &wallet, 600).unwrap();
        // 32 bytes of zeros, base64-encoded — length mismatch must be rejected, not
        // silently coerced.
        use base64::{engine::general_purpose::STANDARD as B64_STD, Engine};
        let bogus = B64_STD.encode([0u8; 32]);
        let err = verify_onboard_submission(secret, &wallet, &msg, &bogus).unwrap_err();
        assert!(err.contains("signature"), "err = {err}");
    }
}
