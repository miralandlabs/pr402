//! Facilitator integration helpers.
//!
//! ## What is always available
//!
//! - **Path constants** and **local** unsigned transaction builders ([`build_exact_spl_payment_tx`],
//!   [`build_sla_escrow_fund_payment_tx`]) used by the serverless binary and tests. **No** extra HTTP
//!   dependencies.
//!
//! ## Optional HTTPS client (`facilitator-http`)
//!
//! Enable Cargo feature **`facilitator-http`** for [`http`] — a small `reqwest` client that mirrors
//! the TypeScript module **`sdk/facilitator-build-tx.ts`** (same paths and JSON shapes). Use this from
//! CLIs, agents, or integration tests. **Do not** enable the feature on the Vercel `facilitator`
//! binary (use **default** features only).

pub use crate::exact_payment_build::{
    build_exact_spl_payment_tx, BuildExactPaymentTxRequest, BuildExactPaymentTxResponse,
    ExactPaymentBuildError,
};
pub use crate::sla_escrow_payment_build::{
    build_sla_escrow_fund_payment_tx, BuildSlaEscrowPaymentTxRequest,
    BuildSlaEscrowPaymentTxResponse, SlaEscrowPaymentBuildError,
};

/// `GET /api/v1/facilitator/supported`
pub const FACILITATOR_SUPPORTED_PATH: &str = "/api/v1/facilitator/supported";

/// `GET /api/v1/facilitator/health`
pub const FACILITATOR_HEALTH_PATH: &str = "/api/v1/facilitator/health";

/// `GET /api/v1/facilitator/capabilities`
pub const FACILITATOR_CAPABILITIES_PATH: &str = "/api/v1/facilitator/capabilities";

/// `POST /api/v1/facilitator/verify`
pub const FACILITATOR_VERIFY_PATH: &str = "/api/v1/facilitator/verify";

/// `POST /api/v1/facilitator/settle`
pub const FACILITATOR_SETTLE_PATH: &str = "/api/v1/facilitator/settle";

/// `POST /api/v1/facilitator/build-exact-payment-tx`
pub const BUILD_EXACT_PAYMENT_TX_PATH: &str = "/api/v1/facilitator/build-exact-payment-tx";

/// `POST /api/v1/facilitator/build-sla-escrow-payment-tx`
pub const BUILD_SLA_ESCROW_PAYMENT_TX_PATH: &str =
    "/api/v1/facilitator/build-sla-escrow-payment-tx";

/// `GET /openapi.json` — OpenAPI 3.1 document (static); also linked from `capabilities.httpEndpoints.openApi`.
pub const FACILITATOR_OPENAPI_PATH: &str = "/openapi.json";

/// `GET /agent-integration.md` — Markdown agent runbook (static `public/agent-integration.md`, like `openapi.json`); linked from `capabilities.httpEndpoints.agentIntegration`.
pub const FACILITATOR_AGENT_INTEGRATION_PATH: &str = "/agent-integration.md";

#[cfg(feature = "facilitator-http")]
pub mod http;
