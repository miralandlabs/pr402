use reqwest::{Client, StatusCode};
use serde_json::Value;
use solana_sdk::{signature::Keypair, signer::Signer, transaction::VersionedTransaction};
use std::fmt;

// ── Error types ─────────────────────────────────────────────────────────

/// Errors returned by [`X402AgentClient::fetch_with_auto_pay`].
///
/// Each variant carries an actionable message so autonomous agents can
/// programmatically decide on remediation (retry, pick different mint,
/// contact Resource Provider, etc.).
#[derive(Debug)]
pub enum X402Error {
    /// The initial GET returned an unexpected HTTP status (not 200 or 402).
    UnexpectedStatus(u16),
    /// The 402 body had no `accepts[]` array — the Resource Provider is misconfigured.
    MissingAccepts,
    /// None of the `accepts[]` entries match the requested mint.
    MintNotAccepted {
        requested_mint: String,
        available_mints: Vec<String>,
    },
    /// The `extra.capabilitiesUrl` field is missing from the chosen `accepts[]` entry.
    /// This means the Resource Provider did not integrate with a Facilitator correctly.
    MissingCapabilitiesUrl,
    /// The Facilitator's `/build-exact-payment-tx` endpoint returned an error.
    BuildFailed {
        status: u16,
        detail: String,
    },
    /// The build response is missing the `verifyBodyTemplate` field.
    MissingVerifyTemplate,
    /// The build response is missing the `transaction` field.
    MissingTransaction,
    /// The agent's wallet pubkey was not found in the transaction's account keys.
    SignerNotInTransaction,
    /// The blockhash embedded in the transaction has expired. Request a fresh build.
    BlockhashExpired {
        expires_at: u64,
    },
    /// Rate limited by the Facilitator. Retry after the indicated duration.
    RateLimited {
        retry_after_secs: u64,
    },
    /// Network or serialization error.
    Transport(String),
}

impl fmt::Display for X402Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedStatus(s) => write!(f, "Unexpected HTTP status {s}. Expected 200 (free) or 402 (payment required)."),
            Self::MissingAccepts => write!(f, "The 402 response has no 'accepts' array. The Resource Provider's payment configuration is invalid. Contact the RP operator."),
            Self::MintNotAccepted { requested_mint, available_mints } => {
                write!(f, "Resource does not accept mint {requested_mint}. Available mints: [{}]. Pick one from this list.",
                    available_mints.join(", "))
            }
            Self::MissingCapabilitiesUrl => write!(f, "This 402-gated resource did not provide extra.capabilitiesUrl. The Resource Provider has not completed Facilitator integration. See docs/SELLER_INTEGRATION.md."),
            Self::BuildFailed { status, detail } => write!(f, "Facilitator build-exact-payment-tx returned HTTP {status}: {detail}"),
            Self::MissingVerifyTemplate => write!(f, "Facilitator response is missing 'verifyBodyTemplate'. The Facilitator may be running an incompatible version."),
            Self::MissingTransaction => write!(f, "Facilitator response is missing 'transaction'. The Facilitator may be running an incompatible version."),
            Self::SignerNotInTransaction => write!(f, "Agent wallet pubkey not found in the unsigned transaction's account keys. The payer address may not match the wallet used to initialize this client."),
            Self::BlockhashExpired { expires_at } => write!(f, "The embedded blockhash expired at UNIX {expires_at}. Request a fresh build from the Facilitator."),
            Self::RateLimited { retry_after_secs } => write!(f, "Facilitator rate-limited this request. Retry after {retry_after_secs}s."),
            Self::Transport(msg) => write!(f, "Network/serialization error: {msg}"),
        }
    }
}

impl std::error::Error for X402Error {}

impl From<reqwest::Error> for X402Error {
    fn from(e: reqwest::Error) -> Self {
        Self::Transport(e.to_string())
    }
}

impl From<bincode::Error> for X402Error {
    fn from(e: bincode::Error) -> Self {
        Self::Transport(format!("bincode: {e}"))
    }
}

impl From<base64::DecodeError> for X402Error {
    fn from(e: base64::DecodeError) -> Self {
        Self::Transport(format!("base64 decode: {e}"))
    }
}

impl From<serde_json::Error> for X402Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Transport(format!("JSON: {e}"))
    }
}

// ── Client ──────────────────────────────────────────────────────────────

/// The primary client for navigating 402 gateways.
pub struct X402AgentClient {
    http: Client,
    wallet: Keypair,
}

impl X402AgentClient {
    pub fn new(wallet: Keypair) -> Self {
        Self {
            http: Client::new(),
            wallet,
        }
    }

    /// Access an API endpoint. If challenged with a 402, automatically routes to the Facilitator,
    /// builds the transaction, signs it, and retries the request fully authorized.
    pub async fn fetch_with_auto_pay(&self, url: &str, preferred_mint: &str) -> Result<reqwest::Response, X402Error> {
        let res = self.http.get(url).send().await?;
        if res.status() == StatusCode::OK {
            return Ok(res);
        } else if res.status() != StatusCode::PAYMENT_REQUIRED {
            return Err(X402Error::UnexpectedStatus(res.status().as_u16()));
        }

        let requirement: Value = res.json().await?;
        let accepts = requirement.get("accepts").and_then(|a| a.as_array())
            .ok_or(X402Error::MissingAccepts)?;

        let available_mints: Vec<String> = accepts.iter()
            .filter_map(|a| a.get("asset").and_then(|x| x.as_str()).map(|s| s.to_string()))
            .collect();

        let rule = accepts.iter().find(|a| a.get("asset").and_then(|x| x.as_str()) == Some(preferred_mint))
            .ok_or_else(|| X402Error::MintNotAccepted {
                requested_mint: preferred_mint.to_string(),
                available_mints,
            })?;

        let cap_url = rule.get("extra").and_then(|e| e.get("capabilitiesUrl")).and_then(|c| c.as_str())
            .ok_or(X402Error::MissingCapabilitiesUrl)?;

        let fac_base_url = cap_url.replace("/capabilities", "");
        let build_url = format!("{}/build-exact-payment-tx", fac_base_url);

        let build_payload = serde_json::json!({
            "payer": self.wallet.pubkey().to_string(),
            "accepted": rule,
            "resource": requirement.get("resource"),
            "skipSourceBalanceCheck": true
        });

        let build_res = self.http.post(&build_url).json(&build_payload).send().await?;
        let build_status = build_res.status().as_u16();

        if build_status == 429 {
            let retry_after = build_res.headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);
            return Err(X402Error::RateLimited { retry_after_secs: retry_after });
        }
        if !build_res.status().is_success() {
            let detail = build_res.text().await.unwrap_or_default();
            return Err(X402Error::BuildFailed { status: build_status, detail });
        }

        let build_json: Value = build_res.json().await?;

        // BUY-3: Check blockhash expiry before signing
        if let Some(expires_at) = build_json.get("recentBlockhashExpiresAt").and_then(|v| v.as_u64()) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now >= expires_at {
                return Err(X402Error::BlockhashExpired { expires_at });
            }
        }

        let mut verify_body = build_json.get("verifyBodyTemplate").cloned()
            .ok_or(X402Error::MissingVerifyTemplate)?;

        let tx_b64 = build_json.get("transaction").and_then(|t| t.as_str())
            .ok_or(X402Error::MissingTransaction)?;

        use base64::{engine::general_purpose::STANDARD, Engine};
        let mut vtx: VersionedTransaction = bincode::deserialize(&STANDARD.decode(tx_b64)?)?;

        // BUY-4: Use payerSignatureIndex if available, otherwise scan keys
        let my_idx = if let Some(idx) = build_json.get("payerSignatureIndex").and_then(|v| v.as_u64()) {
            idx as usize
        } else {
            let my_pubkey = self.wallet.pubkey();
            let keys = vtx.message.static_account_keys();
            keys.iter().position(|k| k == &my_pubkey)
                .ok_or(X402Error::SignerNotInTransaction)?
        };

        vtx.signatures[my_idx] = self.wallet.sign_message(&vtx.message.serialize());
        let signed_tx_b64 = STANDARD.encode(bincode::serialize(&vtx)?);

        verify_body["paymentPayload"]["payload"]["transaction"] = Value::String(signed_tx_b64);
        let proof_b64 = STANDARD.encode(serde_json::to_string(&verify_body)?);

        let final_res = self.http.get(url)
            .header("X-PAYMENT", proof_b64)
            .send().await?;

        Ok(final_res)
    }
}
