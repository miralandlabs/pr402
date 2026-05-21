//! x402 protocol version 2 types.

use crate::chain::ChainId;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::fmt::Display;

use crate::proto;

/// Version 2 of the x402 protocol.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct X402Version2;

impl X402Version2 {
    pub const VALUE: u8 = 2;
}

impl From<X402Version2> for u8 {
    fn from(_: X402Version2) -> Self {
        X402Version2::VALUE
    }
}

impl Serialize for X402Version2 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(Self::VALUE)
    }
}

impl<'de> Deserialize<'de> for X402Version2 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let num = u8::deserialize(deserializer)?;
        if num == Self::VALUE {
            Ok(X402Version2)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected version {}, got {}",
                Self::VALUE,
                num
            )))
        }
    }
}

impl Display for X402Version2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Self::VALUE)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    /// Spec §5.1 orders `url` first in examples.
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyRequest<TPayload, TRequirements> {
    pub x402_version: X402Version2,
    pub payment_payload: TPayload,
    pub payment_requirements: TRequirements,
}

impl<TPayload, TRequirements> VerifyRequest<TPayload, TRequirements>
where
    Self: DeserializeOwned,
{
    pub fn from_proto(
        request: proto::VerifyRequest,
    ) -> Result<Self, proto::PaymentVerificationError> {
        let deserialized: Self = serde_json::from_value(request.into_json())?;
        Ok(deserialized)
    }
}

fn default_payment_extensions() -> serde_json::Value {
    serde_json::json!({})
}

/// x402 v2 §5.2 — field order matches spec examples where practical.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload<TAccepted, TPayload> {
    pub x402_version: X402Version2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceInfo>,
    pub accepted: TAccepted,
    pub payload: TPayload,
    #[serde(default = "default_payment_extensions")]
    pub extensions: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirements<TScheme, TAmount, TAddress, TExtra> {
    pub scheme: TScheme,
    pub network: ChainId,
    pub amount: TAmount,
    pub pay_to: TAddress,
    pub max_timeout_seconds: u64,
    pub asset: TAddress,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<TExtra>,
}

/// Structured representation of a V2 Payment-Required header.
/// This provides proper typing for the payment required response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequired {
    pub x402_version: X402Version2,
    pub resource: ResourceInfo,
    pub accepts: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildPaymentTxResponse {
    pub x402_version: u8,
    pub transaction: String,
    pub recent_blockhash: String,
    pub recent_blockhash_expires_at: u64,
    pub fee_payer: String,
    pub payer: String,
    pub payer_signature_index: usize,
    /// Ordered list of all required signers for the unsigned transaction (base58 pubkeys).
    /// `signerPubkeys[i]` corresponds to `signatures[i]` in the wire transaction. For
    /// single-payer flows this is `[fee_payer]` (index 0 = facilitator-filled) or
    /// `[fee_payer, payer]`; for SLA-Escrow builds the buyer may see additional slots.
    /// Absent on older servers that pre-date the field; tooling should fall back to
    /// `payer_signature_index` when `signer_pubkeys` is empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signer_pubkeys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_uid: Option<String>,
    /// Lower-hex of the on-chain 32-byte `Payment.payment_uid`. For
    /// `payment_uid_hex` callers this echoes the input verbatim. For
    /// legacy string callers (and the ULID auto-mint default) this is
    /// the hex of `sanitize_uid(payment_uid)` — the actual bytes the
    /// builder wrote into FundPayment, suitable for use as the SLA's
    /// `payment_uid` field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_uid_hex: Option<String>,
    pub verify_body_template: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}
