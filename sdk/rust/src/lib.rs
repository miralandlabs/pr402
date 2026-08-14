use reqwest::{Client, StatusCode};
use serde_json::Value;
use solana_sdk::{signature::Keypair, signer::Signer, transaction::VersionedTransaction};
use std::collections::HashSet;
use std::fmt;

const DEFAULT_TRUSTED_FACILITATOR_ORIGINS: &[&str] = &[
    "https://ipay.sh",
    "https://agent.pay402.me",
    "https://preview.ipay.sh",
    "https://preview.agent.pay402.me",
];

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
    BuildFailed { status: u16, detail: String },
    /// The build response is missing the `verifyBodyTemplate` field.
    MissingVerifyTemplate,
    /// The build response is missing the `transaction` field.
    MissingTransaction,
    /// The 402 challenge points at a Facilitator origin the wallet did not trust.
    UntrustedFacilitator(String),
    /// The requested amount exceeds the wallet's configured atomic-unit ceiling.
    PaymentLimitExceeded { amount: u64, maximum: u64 },
    /// The Facilitator changed authoritative terms from the selected 402 line.
    InconsistentBuild(String),
    /// The agent's wallet pubkey was not found in the transaction's account keys.
    SignerNotInTransaction,
    /// The blockhash embedded in the transaction has expired. Request a fresh build.
    BlockhashExpired { expires_at: u64 },
    /// Rate limited by the Facilitator. Retry after the indicated duration.
    RateLimited { retry_after_secs: u64 },
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
            Self::UntrustedFacilitator(origin) => write!(f, "Refusing to send a signing request to untrusted Facilitator origin {origin}. Add it explicitly with with_trusted_facilitator_origins if intended."),
            Self::PaymentLimitExceeded { amount, maximum } => write!(f, "Payment amount {amount} exceeds configured maximum {maximum}."),
            Self::InconsistentBuild(detail) => write!(f, "Facilitator returned an inconsistent payment build: {detail}"),
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
    pub auto_wrap_sol: bool,
    trusted_facilitator_origins: HashSet<String>,
    max_payment_amount: Option<u64>,
}

impl X402AgentClient {
    pub fn new(wallet: Keypair) -> Self {
        Self {
            http: Client::new(),
            wallet,
            auto_wrap_sol: false,
            trusted_facilitator_origins: DEFAULT_TRUSTED_FACILITATOR_ORIGINS
                .iter()
                .map(|origin| (*origin).to_string())
                .collect(),
            max_payment_amount: None,
        }
    }

    pub fn with_auto_wrap_sol(mut self, enabled: bool) -> Self {
        self.auto_wrap_sol = enabled;
        self
    }

    /// Replace the default official-origin allowlist for self-hosted deployments.
    pub fn with_trusted_facilitator_origins<I, S>(mut self, origins: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.trusted_facilitator_origins = origins
            .into_iter()
            .map(Into::into)
            .map(|origin: String| origin.trim_end_matches('/').to_string())
            .collect();
        self
    }

    /// Refuse 402 challenges above this amount, expressed in mint atomic units.
    pub fn with_max_payment_amount(mut self, maximum: u64) -> Self {
        self.max_payment_amount = Some(maximum);
        self
    }

    /// Access an API endpoint. If challenged with a 402, automatically routes to the Facilitator,
    /// builds the transaction, signs it, and retries the request fully authorized.
    pub async fn fetch_with_auto_pay(
        &self,
        url: &str,
        preferred_mint: &str,
    ) -> Result<reqwest::Response, X402Error> {
        let res = self.http.get(url).send().await?;
        if res.status() == StatusCode::OK {
            return Ok(res);
        } else if res.status() != StatusCode::PAYMENT_REQUIRED {
            return Err(X402Error::UnexpectedStatus(res.status().as_u16()));
        }

        let requirement: Value = res.json().await?;
        let accepts = requirement
            .get("accepts")
            .and_then(|a| a.as_array())
            .ok_or(X402Error::MissingAccepts)?;

        let available_mints: Vec<String> = accepts
            .iter()
            .filter(|candidate| is_compatible_exact_solana_rule(candidate))
            .filter_map(|a| {
                a.get("asset")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
            })
            .collect();

        let rule = accepts
            .iter()
            .filter(|candidate| is_compatible_exact_solana_rule(candidate))
            .find(|a| a.get("asset").and_then(|x| x.as_str()) == Some(preferred_mint))
            .ok_or_else(|| X402Error::MintNotAccepted {
                requested_mint: preferred_mint.to_string(),
                available_mints,
            })?;

        let payment_amount = rule
            .get("amount")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| {
                X402Error::InconsistentBuild(
                    "accepted.amount must be a non-negative integer string".to_string(),
                )
            })?;
        if let Some(maximum) = self.max_payment_amount {
            if payment_amount > maximum {
                return Err(X402Error::PaymentLimitExceeded {
                    amount: payment_amount,
                    maximum,
                });
            }
        }

        let cap_url = rule
            .get("extra")
            .and_then(|e| e.get("capabilitiesUrl"))
            .and_then(|c| c.as_str())
            .ok_or(X402Error::MissingCapabilitiesUrl)?;

        let mut fac_base_url = reqwest::Url::parse(cap_url)
            .map_err(|_| X402Error::UntrustedFacilitator(cap_url.to_string()))?;
        let base_path = fac_base_url
            .path()
            .strip_suffix("/capabilities")
            .ok_or_else(|| X402Error::UntrustedFacilitator(cap_url.to_string()))?
            .to_string();
        let origin = fac_base_url.origin().ascii_serialization();
        if !self.trusted_facilitator_origins.contains(&origin) {
            return Err(X402Error::UntrustedFacilitator(origin));
        }
        fac_base_url.set_path(&base_path);
        fac_base_url.set_query(None);
        fac_base_url.set_fragment(None);
        let build_url = format!(
            "{}/build-exact-payment-tx",
            fac_base_url.as_str().trim_end_matches('/')
        );

        let build_payload = serde_json::json!({
            "payer": self.wallet.pubkey().to_string(),
            "accepted": rule,
            "resource": requirement.get("resource"),
            "skipSourceBalanceCheck": true,
            "autoWrapSol": self.auto_wrap_sol
        });

        let build_res = self
            .http
            .post(&build_url)
            .json(&build_payload)
            .send()
            .await?;
        let build_status = build_res.status().as_u16();

        if build_status == 429 {
            let retry_after = build_res
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);
            return Err(X402Error::RateLimited {
                retry_after_secs: retry_after,
            });
        }
        if !build_res.status().is_success() {
            let detail = build_res.text().await.unwrap_or_default();
            return Err(X402Error::BuildFailed {
                status: build_status,
                detail,
            });
        }

        let build_json: Value = build_res.json().await?;

        // BUY-3: Check blockhash expiry before signing
        if let Some(expires_at) = build_json
            .get("recentBlockhashExpiresAt")
            .and_then(|v| v.as_u64())
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now >= expires_at {
                return Err(X402Error::BlockhashExpired { expires_at });
            }
        }

        let mut verify_body = build_json
            .get("verifyBodyTemplate")
            .cloned()
            .ok_or(X402Error::MissingVerifyTemplate)?;

        assert_build_matches_rule(&verify_body, rule)?;

        let tx_b64 = build_json
            .get("transaction")
            .and_then(|t| t.as_str())
            .ok_or(X402Error::MissingTransaction)?;

        use base64::{engine::general_purpose::STANDARD, Engine};
        let mut vtx: VersionedTransaction = bincode::deserialize(&STANDARD.decode(tx_b64)?)?;

        // BUY-4: Use payerSignatureIndex if available, otherwise scan keys
        let my_idx = if let Some(idx) = build_json
            .get("payerSignatureIndex")
            .and_then(|v| v.as_u64())
        {
            idx as usize
        } else {
            let my_pubkey = self.wallet.pubkey();
            let keys = vtx.message.static_account_keys();
            keys.iter()
                .position(|k| k == &my_pubkey)
                .ok_or(X402Error::SignerNotInTransaction)?
        };

        let num_required_signatures = vtx.message.header().num_required_signatures as usize;
        if my_idx >= num_required_signatures
            || vtx.message.static_account_keys().get(my_idx) != Some(&self.wallet.pubkey())
        {
            return Err(X402Error::SignerNotInTransaction);
        }

        vtx.signatures[my_idx] = self.wallet.sign_message(&vtx.message.serialize());
        let signed_tx_b64 = STANDARD.encode(bincode::serialize(&vtx)?);

        let transaction = verify_body
            .pointer_mut("/paymentPayload/payload/transaction")
            .ok_or(X402Error::MissingVerifyTemplate)?;
        *transaction = Value::String(signed_tx_b64);
        let proof_b64 = STANDARD.encode(serde_json::to_string(&verify_body)?);

        // x402 v2 uses the `PAYMENT-SIGNATURE` header name (see the x402 HTTP
        // transport-v2 spec and `public/agent-integration.md` in this repo). v1
        // used `X-PAYMENT`; every seller in this ecosystem today — aethervane,
        // spl-token-balance-serverless, x402-seller-starter — reads only
        // `PAYMENT-SIGNATURE`, so emitting `X-PAYMENT` silently fails with a
        // repeated 402. Emit the canonical v2 header exclusively.
        let final_res = self
            .http
            .get(url)
            .header("PAYMENT-SIGNATURE", proof_b64)
            .send()
            .await?;

        Ok(final_res)
    }
}

fn is_compatible_exact_solana_rule(candidate: &Value) -> bool {
    let scheme = candidate.get("scheme").and_then(Value::as_str);
    let network = candidate.get("network").and_then(Value::as_str);
    matches!(scheme, Some("exact" | "v2:solana:exact"))
        && network.is_some_and(|value| value.starts_with("solana:"))
}

fn normalized_exact_scheme(value: Option<&str>) -> Option<&str> {
    match value {
        Some("v2:solana:exact") => Some("exact"),
        other => other,
    }
}

fn assert_build_matches_rule(template: &Value, rule: &Value) -> Result<(), X402Error> {
    let built = template
        .get("paymentRequirements")
        .and_then(Value::as_object)
        .ok_or(X402Error::MissingVerifyTemplate)?;
    let selected = rule.as_object().ok_or_else(|| {
        X402Error::InconsistentBuild("selected accepts[] entry is not an object".to_string())
    })?;

    for field in ["scheme", "network", "asset", "amount", "payTo"] {
        let matches = if field == "scheme" {
            normalized_exact_scheme(selected.get(field).and_then(Value::as_str))
                == normalized_exact_scheme(built.get(field).and_then(Value::as_str))
        } else {
            selected.get(field) == built.get(field)
        };
        if !matches {
            return Err(X402Error::InconsistentBuild(format!(
                "Facilitator changed payment term '{field}' in verifyBodyTemplate"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_only_exact_solana_rules() {
        assert!(is_compatible_exact_solana_rule(&serde_json::json!({
            "scheme": "v2:solana:exact",
            "network": "solana:mainnet"
        })));
        assert!(!is_compatible_exact_solana_rule(&serde_json::json!({
            "scheme": "sla-escrow",
            "network": "solana:mainnet"
        })));
        assert!(!is_compatible_exact_solana_rule(&serde_json::json!({
            "scheme": "exact",
            "network": "eip155:1"
        })));
    }

    #[test]
    fn accepts_scheme_alias_but_rejects_changed_terms() {
        let selected = serde_json::json!({
            "scheme": "v2:solana:exact",
            "network": "solana:mainnet",
            "asset": "mint",
            "amount": "10",
            "payTo": "vault"
        });
        let matching = serde_json::json!({
            "paymentRequirements": {
                "scheme": "exact",
                "network": "solana:mainnet",
                "asset": "mint",
                "amount": "10",
                "payTo": "vault"
            }
        });
        assert_build_matches_rule(&matching, &selected).unwrap();

        let changed = serde_json::json!({
            "paymentRequirements": {
                "scheme": "exact",
                "network": "solana:mainnet",
                "asset": "mint",
                "amount": "11",
                "payTo": "vault"
            }
        });
        assert!(matches!(
            assert_build_matches_rule(&changed, &selected),
            Err(X402Error::InconsistentBuild(_))
        ));
    }
}
