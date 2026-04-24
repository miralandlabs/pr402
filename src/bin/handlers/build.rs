use super::*;

pub async fn handle_build_exact_payment_tx(body: Body) -> Response<Body> {
    let cp = match chain_provider_for_build() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let req: pr402::exact_payment_build::BuildExactPaymentTxRequest =
        match serde_json::from_str(&body_str) {
            Ok(r) => r,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e))
            }
        };
    match pr402::exact_payment_build::build_exact_spl_payment_tx(&cp.solana, pr402_db(), req).await
    {
        Ok(out) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(
                serde_json::to_string(&out)
                    .unwrap_or_else(|_| r#"{"error":"serialize failed"}"#.into()),
            ))
            .unwrap(),
        Err(e) => {
            let status = match e {
                pr402::exact_payment_build::ExactPaymentBuildError::NetworkMismatch { .. }
                | pr402::exact_payment_build::ExactPaymentBuildError::InvalidRequest(_) => {
                    StatusCode::BAD_REQUEST
                }
                pr402::exact_payment_build::ExactPaymentBuildError::Unsupported(_) => {
                    StatusCode::NOT_IMPLEMENTED
                }
                pr402::exact_payment_build::ExactPaymentBuildError::Rpc(_) => {
                    StatusCode::BAD_GATEWAY
                }
            };
            error_response(status, &e.to_string())
        }
    }
}

/// Build an unsigned `v2:solana:sla-escrow` [`FundPayment`] transaction for buyer signing.
/// See [`pr402::sla_escrow_payment_build`].
pub async fn handle_build_sla_escrow_payment_tx(body: Body) -> Response<Body> {
    let cp = match chain_provider_for_build() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let req: pr402::sla_escrow_payment_build::BuildSlaEscrowPaymentTxRequest =
        match serde_json::from_str(&body_str) {
            Ok(r) => r,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e))
            }
        };
    if req.facilitator_pays_transaction_fees && !cp.sla_escrow_allow_facilitator_fee_sponsorship {
        return error_response(
            StatusCode::BAD_REQUEST,
            "SLA-Escrow facilitator-paid Solana fees are disabled on this deployment. Set PR402_SLA_ESCROW_ALLOW_FACILITATOR_FEE_SPONSORSHIP to true (or 1) to allow facilitatorPaysTransactionFees: true.",
        );
    }
    match pr402::sla_escrow_payment_build::build_sla_escrow_fund_payment_tx(
        &cp.solana,
        pr402_db(),
        req,
    )
    .await
    {
        Ok(out) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(
                serde_json::to_string(&out)
                    .unwrap_or_else(|_| r#"{"error":"serialize failed"}"#.into()),
            ))
            .unwrap(),
        Err(e) => {
            let status = match e {
                pr402::sla_escrow_payment_build::SlaEscrowPaymentBuildError::NetworkMismatch {
                    ..
                }
                | pr402::sla_escrow_payment_build::SlaEscrowPaymentBuildError::InvalidRequest(_) => {
                    StatusCode::BAD_REQUEST
                }
                pr402::sla_escrow_payment_build::SlaEscrowPaymentBuildError::Unsupported(_) => {
                    StatusCode::NOT_IMPLEMENTED
                }
                pr402::sla_escrow_payment_build::SlaEscrowPaymentBuildError::NotConfigured => {
                    StatusCode::NOT_IMPLEMENTED
                }
                pr402::sla_escrow_payment_build::SlaEscrowPaymentBuildError::Rpc(_) => {
                    StatusCode::BAD_GATEWAY
                }
            };
            error_response(status, &e.to_string())
        }
    }
}
