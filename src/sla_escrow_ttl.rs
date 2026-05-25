//! SLA-Escrow `FundPayment` TTL rules (`x402/sla-escrow-fund-payment-ttl/v1`).
//!
//! Applies to every `scheme: "sla-escrow"` resource provider and facilitator verify path —
//! not a single product binding.

use {
    crate::parameters::{
        self, PR402_SLA_ESCROW_DELIVERY_BUDGET_SECONDS, PR402_SLA_ESCROW_DELIVERY_CUTOFF_SECONDS,
    },
    sla_escrow_api::consts::{DEFAULT_DELIVERY_CUTOFF_SECONDS, MIN_TTL_SECONDS},
    std::fmt,
};

/// Normative identifier for integrators.
pub const RULE_ID: &str = "x402/sla-escrow-fund-payment-ttl/v1";

/// Default post-funding work budget (verify/settle + delivery + registry + SubmitDelivery).
/// Override via DB `parameters` row or env `PR402_SLA_ESCROW_DELIVERY_BUDGET_SECONDS`.
pub const DEFAULT_DELIVERY_BUDGET_SECONDS: i64 = 300;

/// Minimum allowed `FundPayment.ttl_seconds` / `accepts[].maxTimeoutSeconds` for a deployment.
pub fn min_fund_payment_ttl_seconds(
    delivery_cutoff_seconds: i64,
    delivery_budget_seconds: i64,
) -> u64 {
    let min = delivery_cutoff_seconds
        .saturating_add(delivery_budget_seconds)
        .max(MIN_TTL_SECONDS);
    min as u64
}

/// **Resolution:** DB `parameters` row (when cache is warm) → env var → on-chain default const.
///
/// Call [`parameters::refresh_parameters_from_db`] on request entrypoints before using this sync resolver.
pub fn resolve_delivery_cutoff_seconds() -> i64 {
    parameters::resolve_u64_sync(
        PR402_SLA_ESCROW_DELIVERY_CUTOFF_SECONDS,
        PR402_SLA_ESCROW_DELIVERY_CUTOFF_SECONDS,
        DEFAULT_DELIVERY_CUTOFF_SECONDS.max(0) as u64,
    ) as i64
}

/// See [`resolve_delivery_cutoff_seconds`].
pub fn resolve_delivery_budget_seconds() -> i64 {
    parameters::resolve_u64_sync(
        PR402_SLA_ESCROW_DELIVERY_BUDGET_SECONDS,
        PR402_SLA_ESCROW_DELIVERY_BUDGET_SECONDS,
        DEFAULT_DELIVERY_BUDGET_SECONDS.max(0) as u64,
    ) as i64
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FundPaymentTtlError {
    TtlMismatch {
        quoted_max_timeout_seconds: u64,
        fund_payment_ttl_seconds: u64,
    },
    TtlTooShort {
        fund_payment_ttl_seconds: u64,
        minimum_required_seconds: u64,
        delivery_cutoff_seconds: i64,
        delivery_budget_seconds: i64,
    },
}

impl fmt::Display for FundPaymentTtlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TtlMismatch {
                quoted_max_timeout_seconds,
                fund_payment_ttl_seconds,
            } => write!(
                f,
                "FundPayment.ttl_seconds ({fund_payment_ttl_seconds}) must equal accepts[].maxTimeoutSeconds ({quoted_max_timeout_seconds})"
            ),
            Self::TtlTooShort {
                fund_payment_ttl_seconds,
                minimum_required_seconds,
                delivery_cutoff_seconds,
                delivery_budget_seconds,
            } => write!(
                f,
                "FundPayment.ttl_seconds ({fund_payment_ttl_seconds}) is below minimum {minimum_required_seconds} \
                 (delivery_cutoff_seconds={delivery_cutoff_seconds} + delivery_budget_seconds={delivery_budget_seconds})"
            ),
        }
    }
}

impl std::error::Error for FundPaymentTtlError {}

/// Validate TTL binding for sla-escrow verify/build.
pub fn validate_fund_payment_ttl(
    fund_payment_ttl_seconds: u64,
    quoted_max_timeout_seconds: u64,
    delivery_cutoff_seconds: i64,
    delivery_budget_seconds: i64,
) -> Result<(), FundPaymentTtlError> {
    if fund_payment_ttl_seconds != quoted_max_timeout_seconds {
        return Err(FundPaymentTtlError::TtlMismatch {
            quoted_max_timeout_seconds,
            fund_payment_ttl_seconds,
        });
    }
    let minimum = min_fund_payment_ttl_seconds(delivery_cutoff_seconds, delivery_budget_seconds);
    if fund_payment_ttl_seconds < minimum {
        return Err(FundPaymentTtlError::TtlTooShort {
            fund_payment_ttl_seconds,
            minimum_required_seconds: minimum,
            delivery_cutoff_seconds,
            delivery_budget_seconds,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_ttl_is_cutoff_plus_budget() {
        assert_eq!(min_fund_payment_ttl_seconds(300, 300), 600);
        assert_eq!(min_fund_payment_ttl_seconds(300, 0), 300);
    }

    #[test]
    fn rejects_mismatch_and_short_ttl() {
        assert!(validate_fund_payment_ttl(60, 60, 300, 300).is_err());
        assert!(validate_fund_payment_ttl(3600, 300, 300, 300)
            .unwrap_err()
            .to_string()
            .contains("must equal"));
        assert!(validate_fund_payment_ttl(3600, 3600, 300, 300).is_ok());
    }
}
