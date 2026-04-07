//! HTTPS client for a deployed pr402 facilitator — mirrors `sdk/facilitator-build-tx.ts` at the repo root.
//!
//! Enable Cargo feature **`facilitator-http`**. Paths match the constants in [`super`](crate::sdk).

use std::sync::OnceLock;

use reqwest::header::CONTENT_TYPE;
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::{
    BuildExactPaymentTxRequest, BuildExactPaymentTxResponse, BuildSlaEscrowPaymentTxRequest,
    BuildSlaEscrowPaymentTxResponse, BUILD_EXACT_PAYMENT_TX_PATH, BUILD_SLA_ESCROW_PAYMENT_TX_PATH,
    FACILITATOR_AGENT_INTEGRATION_PATH, FACILITATOR_CAPABILITIES_PATH, FACILITATOR_HEALTH_PATH,
    FACILITATOR_OPENAPI_PATH, FACILITATOR_SETTLE_PATH, FACILITATOR_SUPPORTED_PATH,
    FACILITATOR_VERIFY_PATH,
};

fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .build()
            .expect("reqwest Client::builder().build()")
    })
}

/// Normalize facilitator origin: trim trailing `/`, reject empty.
pub fn normalize_base_url(base_url: &str) -> Result<String, FacilitatorHttpError> {
    let s = base_url.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err(FacilitatorHttpError::InvalidBaseUrl(
            "empty facilitator base URL".into(),
        ));
    }
    Ok(s.to_string())
}

fn join_url(base: &str, path: &str) -> String {
    format!("{}{}", base, path)
}

/// Errors from facilitator HTTP calls (network, non-2xx, JSON).
#[derive(Debug, thiserror::Error)]
pub enum FacilitatorHttpError {
    #[error("invalid facilitator base URL: {0}")]
    InvalidBaseUrl(String),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error("{path}: HTTP {status}: {body}")]
    Unsuccessful {
        path: String,
        status: u16,
        body: String,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

async fn get_text(base_url: &str, path: &str) -> Result<String, FacilitatorHttpError> {
    let base = normalize_base_url(base_url)?;
    let url = join_url(&base, path);
    let res = shared_client().get(url).send().await?;
    let status = res.status();
    let body = res.text().await?;
    if !status.is_success() {
        return Err(FacilitatorHttpError::Unsuccessful {
            path: path.to_string(),
            status: status.as_u16(),
            body,
        });
    }
    Ok(body)
}

async fn get_json_value(
    base_url: &str,
    path: &str,
) -> Result<serde_json::Value, FacilitatorHttpError> {
    let text = get_text(base_url, path).await?;
    Ok(serde_json::from_str(&text)?)
}

async fn post_json<T: DeserializeOwned>(
    base_url: &str,
    path: &str,
    body: &impl Serialize,
    correlation_id: Option<&str>,
) -> Result<T, FacilitatorHttpError> {
    let base = normalize_base_url(base_url)?;
    let url = join_url(&base, path);
    let mut req = shared_client()
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .json(body);
    if let Some(id) = correlation_id {
        req = req.header("X-Correlation-ID", id);
    }
    let res = req.send().await?;
    let status = res.status();
    let text = res.text().await?;
    if !status.is_success() {
        return Err(FacilitatorHttpError::Unsuccessful {
            path: path.to_string(),
            status: status.as_u16(),
            body: text,
        });
    }
    Ok(serde_json::from_str(&text)?)
}

/// Reusable handle: stores normalized base URL (no trailing slash).
#[derive(Clone, Debug)]
pub struct FacilitatorHttpClient {
    base_url: String,
}

impl FacilitatorHttpClient {
    /// `base_url`: e.g. `https://preview.agent.pay402.me` (trailing slash OK).
    pub fn new(base_url: impl AsRef<str>) -> Result<Self, FacilitatorHttpError> {
        Ok(Self {
            base_url: normalize_base_url(base_url.as_ref())?,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn fetch_openapi(&self) -> Result<serde_json::Value, FacilitatorHttpError> {
        fetch_facilitator_openapi(&self.base_url).await
    }

    pub async fn fetch_agent_integration_markdown(&self) -> Result<String, FacilitatorHttpError> {
        fetch_agent_integration_markdown(&self.base_url).await
    }

    pub async fn supported(&self) -> Result<serde_json::Value, FacilitatorHttpError> {
        get_supported(&self.base_url).await
    }

    pub async fn health(&self) -> Result<serde_json::Value, FacilitatorHttpError> {
        get_health(&self.base_url).await
    }

    pub async fn capabilities(&self) -> Result<serde_json::Value, FacilitatorHttpError> {
        get_capabilities(&self.base_url).await
    }

    pub async fn verify_payment(
        &self,
        body: &serde_json::Value,
        correlation_id: Option<&str>,
    ) -> Result<serde_json::Value, FacilitatorHttpError> {
        verify_payment(&self.base_url, body, correlation_id).await
    }

    pub async fn settle_payment(
        &self,
        body: &serde_json::Value,
        correlation_id: Option<&str>,
    ) -> Result<serde_json::Value, FacilitatorHttpError> {
        settle_payment(&self.base_url, body, correlation_id).await
    }

    pub async fn build_exact_payment_tx(
        &self,
        body: &BuildExactPaymentTxRequest,
    ) -> Result<BuildExactPaymentTxResponse, FacilitatorHttpError> {
        build_exact_payment_tx(&self.base_url, body).await
    }

    pub async fn build_sla_escrow_payment_tx(
        &self,
        body: &BuildSlaEscrowPaymentTxRequest,
    ) -> Result<BuildSlaEscrowPaymentTxResponse, FacilitatorHttpError> {
        build_sla_escrow_payment_tx(&self.base_url, body).await
    }
}

// --- Free functions (parity with `facilitator-build-tx.ts`) ---

/// `GET /openapi.json`
pub async fn fetch_facilitator_openapi(
    facilitator_base_url: &str,
) -> Result<serde_json::Value, FacilitatorHttpError> {
    get_json_value(facilitator_base_url, FACILITATOR_OPENAPI_PATH).await
}

/// Markdown runbook (`GET /agent-integration.md`).
pub async fn fetch_agent_integration_markdown(
    facilitator_base_url: &str,
) -> Result<String, FacilitatorHttpError> {
    get_text(facilitator_base_url, FACILITATOR_AGENT_INTEGRATION_PATH).await
}

/// `GET .../supported`
pub async fn get_supported(
    facilitator_base_url: &str,
) -> Result<serde_json::Value, FacilitatorHttpError> {
    get_json_value(facilitator_base_url, FACILITATOR_SUPPORTED_PATH).await
}

/// `GET .../health`
pub async fn get_health(
    facilitator_base_url: &str,
) -> Result<serde_json::Value, FacilitatorHttpError> {
    get_json_value(facilitator_base_url, FACILITATOR_HEALTH_PATH).await
}

/// `GET .../capabilities`
pub async fn get_capabilities(
    facilitator_base_url: &str,
) -> Result<serde_json::Value, FacilitatorHttpError> {
    get_json_value(facilitator_base_url, FACILITATOR_CAPABILITIES_PATH).await
}

/// `POST .../verify` — optional `X-Correlation-ID` header.
pub async fn verify_payment(
    facilitator_base_url: &str,
    body: &serde_json::Value,
    correlation_id: Option<&str>,
) -> Result<serde_json::Value, FacilitatorHttpError> {
    post_json(
        facilitator_base_url,
        FACILITATOR_VERIFY_PATH,
        body,
        correlation_id,
    )
    .await
}

/// `POST .../settle` — reuse the same body and correlation id as verify.
pub async fn settle_payment(
    facilitator_base_url: &str,
    body: &serde_json::Value,
    correlation_id: Option<&str>,
) -> Result<serde_json::Value, FacilitatorHttpError> {
    post_json(
        facilitator_base_url,
        FACILITATOR_SETTLE_PATH,
        body,
        correlation_id,
    )
    .await
}

/// `POST .../build-exact-payment-tx`
pub async fn build_exact_payment_tx(
    facilitator_base_url: &str,
    body: &BuildExactPaymentTxRequest,
) -> Result<BuildExactPaymentTxResponse, FacilitatorHttpError> {
    post_json(
        facilitator_base_url,
        BUILD_EXACT_PAYMENT_TX_PATH,
        body,
        None,
    )
    .await
}

/// `POST .../build-sla-escrow-payment-tx`
pub async fn build_sla_escrow_payment_tx(
    facilitator_base_url: &str,
    body: &BuildSlaEscrowPaymentTxRequest,
) -> Result<BuildSlaEscrowPaymentTxResponse, FacilitatorHttpError> {
    post_json(
        facilitator_base_url,
        BUILD_SLA_ESCROW_PAYMENT_TX_PATH,
        body,
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::normalize_base_url;

    #[test]
    fn normalize_trims_slash() {
        assert_eq!(
            normalize_base_url("https://example.com/").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            normalize_base_url("https://example.com").unwrap(),
            "https://example.com"
        );
    }

    #[test]
    fn normalize_rejects_empty() {
        assert!(normalize_base_url("").is_err());
        assert!(normalize_base_url("  /  ").is_err());
    }
}
