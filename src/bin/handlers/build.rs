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
    // Warm the parameters cache so the strict oracleProfiles[] check (when
    // enabled via PR402_SLA_ESCROW_REQUIRE_PROFILE_MATCH) reads the current
    // facilitator-advertised profile ids from the DB. No-op when no DB.
    pr402::parameters::refresh_parameters_from_db(pr402_db()).await;
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
                pr402::sla_escrow_payment_build::SlaEscrowPaymentBuildError::OracleUnhealthy(_) => {
                    StatusCode::SERVICE_UNAVAILABLE
                }
            };
            error_response(status, &e.to_string())
        }
    }
}

pub async fn handle_build_oracle_confirm_tx(body: Body) -> Response<Body> {
    let cp = match chain_provider_for_build() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let req: pr402::sla_escrow_payment_build::BuildOracleConfirmTxRequest =
        match serde_json::from_str(&body_str) {
            Ok(r) => r,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e))
            }
        };
    match pr402::sla_escrow_payment_build::build_oracle_confirm_tx(&cp.solana, req).await {
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
                pr402::sla_escrow_payment_build::SlaEscrowPaymentBuildError::OracleUnhealthy(_) => {
                    // The oracle confirm path doesn't currently emit this
                    // variant; mapping is included for match exhaustiveness.
                    StatusCode::SERVICE_UNAVAILABLE
                }
            };
            error_response(status, &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// v0.5 extended-payment endpoints (additive; ix 7–12). Thin wrappers over the
// builders in `pr402::sla_escrow_payment_build`. Each returns an unsigned,
// client-signed tx. The v0.4 endpoints are untouched.
// ---------------------------------------------------------------------------

/// Map a build error to its HTTP status (shared by the v0.5 action endpoints).
fn sla_v2_build_status(
    e: &pr402::sla_escrow_payment_build::SlaEscrowPaymentBuildError,
) -> StatusCode {
    use pr402::sla_escrow_payment_build::SlaEscrowPaymentBuildError as E;
    match e {
        E::NetworkMismatch { .. } | E::InvalidRequest(_) => StatusCode::BAD_REQUEST,
        E::Unsupported(_) | E::NotConfigured => StatusCode::NOT_IMPLEMENTED,
        E::Rpc(_) => StatusCode::BAD_GATEWAY,
        E::OracleUnhealthy(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn sla_v2_json_ok<T: serde::Serialize>(out: &T) -> Response<Body> {
    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(
            serde_json::to_string(out).unwrap_or_else(|_| r#"{"error":"serialize failed"}"#.into()),
        ))
        .unwrap()
}

/// `POST /api/v1/facilitator/build-sla-escrow-payment-v2-tx` — FundPaymentV2 (ix 8).
pub async fn handle_build_sla_escrow_payment_v2_tx(body: Body) -> Response<Body> {
    let cp = match chain_provider_for_build() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let req: pr402::sla_escrow_payment_build::BuildFundPaymentV2TxRequest =
        match serde_json::from_str(&body_str) {
            Ok(r) => r,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e))
            }
        };
    match pr402::sla_escrow_payment_build::build_fund_payment_v2_tx(&cp.solana, req).await {
        Ok(out) => sla_v2_json_ok(&out),
        Err(e) => error_response(sla_v2_build_status(&e), &e.to_string()),
    }
}

/// `POST /api/v1/facilitator/build-sla-escrow-approve-tx` — ApproveDelivery (ix 7).
pub async fn handle_build_sla_escrow_approve_tx(body: Body) -> Response<Body> {
    let cp = match chain_provider_for_build() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let req: pr402::sla_escrow_payment_build::BuildApproveDeliveryTxRequest =
        match serde_json::from_str(&body_str) {
            Ok(r) => r,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e))
            }
        };
    match pr402::sla_escrow_payment_build::build_approve_delivery_tx(&cp.solana, req).await {
        Ok(out) => sla_v2_json_ok(&out),
        Err(e) => error_response(sla_v2_build_status(&e), &e.to_string()),
    }
}

/// `POST /api/v1/facilitator/build-sla-escrow-dispute-tx` — DisputePayment (ix 11).
pub async fn handle_build_sla_escrow_dispute_tx(body: Body) -> Response<Body> {
    let cp = match chain_provider_for_build() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let req: pr402::sla_escrow_payment_build::BuildDisputePaymentTxRequest =
        match serde_json::from_str(&body_str) {
            Ok(r) => r,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e))
            }
        };
    match pr402::sla_escrow_payment_build::build_dispute_payment_tx(&cp.solana, req).await {
        Ok(out) => sla_v2_json_ok(&out),
        Err(e) => error_response(sla_v2_build_status(&e), &e.to_string()),
    }
}

/// `POST /api/v1/facilitator/build-sla-escrow-mutual-action-tx` — Propose/Accept (ix 9/10).
pub async fn handle_build_sla_escrow_mutual_action_tx(body: Body) -> Response<Body> {
    let cp = match chain_provider_for_build() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let req: pr402::sla_escrow_payment_build::BuildMutualActionTxRequest =
        match serde_json::from_str(&body_str) {
            Ok(r) => r,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e))
            }
        };
    match pr402::sla_escrow_payment_build::build_mutual_action_tx(&cp.solana, req).await {
        Ok(out) => sla_v2_json_ok(&out),
        Err(e) => error_response(sla_v2_build_status(&e), &e.to_string()),
    }
}

/// `POST /api/v1/facilitator/build-sla-escrow-resolve-split-tx` — ResolveWithSplit (ix 12).
pub async fn handle_build_sla_escrow_resolve_split_tx(body: Body) -> Response<Body> {
    let cp = match chain_provider_for_build() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let req: pr402::sla_escrow_payment_build::BuildResolveWithSplitTxRequest =
        match serde_json::from_str(&body_str) {
            Ok(r) => r,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e))
            }
        };
    match pr402::sla_escrow_payment_build::build_resolve_with_split_tx(&cp.solana, req).await {
        Ok(out) => sla_v2_json_ok(&out),
        Err(e) => error_response(sla_v2_build_status(&e), &e.to_string()),
    }
}

/// Build an unsigned refund transaction (merchant → payer `TransferChecked`).
/// See [`pr402::refund_tx_build`].
pub async fn handle_build_refund_tx(body: Body) -> Response<Body> {
    let cp = match chain_provider_for_build() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let req: pr402::refund_tx_build::BuildRefundTxRequest = match serde_json::from_str(&body_str) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };
    match pr402::refund_tx_build::build_refund_tx(&cp.solana, pr402_db(), req).await {
        Ok(out) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(
                serde_json::to_string(&out)
                    .unwrap_or_else(|_| r#"{"error":"serialize failed"}"#.into()),
            ))
            .unwrap(),
        Err(e) => {
            let status = match &e {
                pr402::refund_tx_build::RefundTxBuildError::InvalidRequest(_) => {
                    StatusCode::BAD_REQUEST
                }
                pr402::refund_tx_build::RefundTxBuildError::Rpc(_) => StatusCode::BAD_GATEWAY,
            };
            error_response(status, &e.to_string())
        }
    }
}
