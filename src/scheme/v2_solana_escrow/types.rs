//! Types for v2:solana:escrow scheme.

use std::fmt;

use crate::chain::solana::Address;
use crate::proto::util::U64String;
use crate::proto::v2;
use serde::{Deserialize, Serialize};

/// SLAEscrow scheme identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SLAEscrowScheme;

impl AsRef<str> for SLAEscrowScheme {
    fn as_ref(&self) -> &str {
        "sla-escrow"
    }
}

impl fmt::Display for SLAEscrowScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sla-escrow")
    }
}

/// Solana escrow payload (base64-encoded transaction).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SLAEscrowSolanaPayload {
    pub transaction: String,
}

/// Supported payment kind extra information for escrow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SLAEscrowPaymentRequirementsExtra {
    pub fee_payer: Address,
    pub oracle_authority: Address,
    pub escrow_program_id: Address,
    pub ttl_seconds: u64,
}

/// Verify request for v2:solana:escrow.
pub type VerifyRequest = v2::VerifyRequest<PaymentPayload, PaymentRequirements>;

/// Settle request (same as verify).
pub type SettleRequest = VerifyRequest;

/// Payment payload for v2:solana:escrow.
pub type PaymentPayload = v2::PaymentPayload<PaymentRequirements, SLAEscrowSolanaPayload>;

/// Payment requirements for v2:solana:escrow.
pub type PaymentRequirements =
    v2::PaymentRequirements<SLAEscrowScheme, U64String, Address, SLAEscrowPaymentRequirementsExtra>;
