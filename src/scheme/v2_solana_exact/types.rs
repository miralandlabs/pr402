//! Types for v2:solana:exact scheme.

use std::fmt;

use crate::chain::solana::Address;
use crate::proto::util::{U16String, U64String};
use crate::proto::v2;
use crate::{chain::solana::SolanaChainProvider, proto, scheme::X402SchemeFacilitatorError};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Exact scheme identifier.
///
/// Wire format is the string `"exact"` (x402 `PaymentRequirements.scheme`), not a unit JSON value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactScheme;

impl Serialize for ExactScheme {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(ExactScheme.as_ref())
    }
}

impl<'de> Deserialize<'de> for ExactScheme {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        if s == ExactScheme.as_ref() {
            Ok(ExactScheme)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected scheme {:?}, got {:?}",
                ExactScheme.as_ref(),
                s
            )))
        }
    }
}

impl AsRef<str> for ExactScheme {
    fn as_ref(&self) -> &str {
        "exact"
    }
}

impl fmt::Display for ExactScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("exact")
    }
}

/// Solana exact payload (base64-encoded transaction).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactSolanaPayload {
    pub transaction: String,
}

/// Supported payment kind extra information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SupportedPaymentKindExtra {
    pub fee_payer: Address,
    pub program_id: Address,
    pub config_address: Address,
    pub fee_bps: U16String,
    pub min_fee_amount: U64String,     // New: notify buyer of SPL floor
    pub min_fee_amount_sol: U64String, // New: notify buyer of SOL floor
    #[serde(default)]
    pub merchant_wallet: Option<Address>, // IDENTITY: Original wallet for sweep/audit
    #[serde(default)]
    pub beneficiary: Option<Address>, // COLLECTION: Final sweep destination (Priority)
}

/// Verify request for v2:solana:exact.
pub type VerifyRequest = v2::VerifyRequest<PaymentPayload, PaymentRequirements>;

/// Settle request (same as verify).
pub type SettleRequest = VerifyRequest;

/// Payment payload for v2:solana:exact.
pub type PaymentPayload = v2::PaymentPayload<PaymentRequirements, ExactSolanaPayload>;

/// Payment requirements for v2:solana:exact.
pub type PaymentRequirements =
    v2::PaymentRequirements<ExactScheme, U64String, Address, SupportedPaymentKindExtra>;

/// Elevate a raw merchant wallet to its institutional SplitVault PDA during discovery.
pub fn v2_upgrade(
    request: &proto::PaymentRequired,
    provider: &SolanaChainProvider,
) -> Result<v2::PaymentRequired, X402SchemeFacilitatorError> {
    let mut pr = match request {
        proto::PaymentRequired::V2(v2) => v2.clone(),
    };

    let chain_id_str = provider.chain_id().to_string();

    for accept in pr.accepts.iter_mut() {
        let scheme = accept.get("scheme").and_then(|s| s.as_str());
        let network = accept.get("network").and_then(|n| n.as_str());

        if scheme == Some(ExactScheme.as_ref()) && network == Some(&chain_id_str) {
            let pay_to = accept.get("payTo").and_then(|p| p.as_str()).unwrap_or("");
            if let Ok(merchant_wallet) = Pubkey::from_str(pay_to) {
                // Determine the Vault PDA for this merchant
                let (vault_pda, _) = provider.get_vault_pda(&merchant_wallet);

                if let Some(obj) = accept.as_object_mut() {
                    obj.insert(
                        "payTo".to_string(),
                        serde_json::json!(vault_pda.to_string()),
                    );

                    // Inject Facilitator institutional metadata
                    if let Some(us_config) = provider.universalsettle() {
                        let (config_address, _) = provider.get_config_pda(&us_config.program_id);
                        let extra = SupportedPaymentKindExtra {
                            fee_payer: provider.fee_payer().into(),
                            program_id: us_config.program_id.into(),
                            config_address: config_address.into(),
                            fee_bps: us_config.fee_bps.unwrap_or(0).into(),
                            min_fee_amount: us_config.min_fee_amount.unwrap_or(0).into(),
                            min_fee_amount_sol: us_config.min_fee_amount_sol.unwrap_or(0).into(),
                            merchant_wallet: Some(merchant_wallet.into()),
                            beneficiary: obj
                                .get("beneficiary")
                                .and_then(|v| v.as_str())
                                .and_then(|s| solana_pubkey::Pubkey::from_str(s).ok())
                                .map(Address::new),
                        };
                        obj.insert("extra".to_string(), serde_json::to_value(extra).unwrap());
                    }
                }
            }
        }
    }

    Ok(pr)
}
