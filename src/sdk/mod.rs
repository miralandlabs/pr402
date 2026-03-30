//! Thin client helpers for facilitator HTTP APIs (paths + re-exported request/response types).

pub use crate::exact_payment_build::{
    build_exact_spl_payment_tx, BuildExactPaymentTxRequest, BuildExactPaymentTxResponse,
    ExactPaymentBuildError,
};
pub use crate::sla_escrow_payment_build::{
    build_sla_escrow_fund_payment_tx, BuildSlaEscrowPaymentTxRequest,
    BuildSlaEscrowPaymentTxResponse, SlaEscrowPaymentBuildError,
};

/// `POST /api/v1/facilitator/build-exact-payment-tx`
pub const BUILD_EXACT_PAYMENT_TX_PATH: &str = "/api/v1/facilitator/build-exact-payment-tx";

/// `POST /api/v1/facilitator/build-sla-escrow-payment-tx`
pub const BUILD_SLA_ESCROW_PAYMENT_TX_PATH: &str =
    "/api/v1/facilitator/build-sla-escrow-payment-tx";
