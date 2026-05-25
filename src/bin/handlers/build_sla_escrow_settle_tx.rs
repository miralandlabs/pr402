//! Manual settle affordance — build unsigned Release/Refund tx for a payment.

use super::*;

use pr402::settlement_keeper::sla_escrow::build_settle_tx_for_payment;
use pr402::settlement_keeper::types::{
    BuildSlaEscrowSettleTxRequest, BuildSlaEscrowSettleTxResponse,
};

/// Public endpoint: no bearer auth. Returns a facilitator-signed tx the buyer
/// can broadcast immediately (permissionless settlement path only).
pub async fn handle_build_sla_escrow_settle_tx(body: Body) -> Response<Body> {
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => "{}".to_string(),
    };
    let req: BuildSlaEscrowSettleTxRequest = match serde_json::from_str(&body_str) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };

    let cp = match CHAIN_PROVIDER.get() {
        Some(c) => c,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "chain provider not initialized",
            );
        }
    };

    let escrow_config = match cp.solana.sla_escrow() {
        Some(e) => e,
        None => return error_response(StatusCode::BAD_REQUEST, "SLA-Escrow not configured"),
    };

    match build_settle_tx_for_payment(cp, escrow_config.program_id, &req.payment_uid_hex).await {
        Ok((action, unsigned_tx, payment_pda)) => {
            let res = BuildSlaEscrowSettleTxResponse {
                action,
                unsigned_transaction: unsigned_tx,
                payment_uid_hex: req.payment_uid_hex,
                payment_pda: payment_pda.to_string(),
            };
            facilitator_response!()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::Text(serde_json::to_string(&res).unwrap_or_default()))
                .unwrap()
        }
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e),
    }
}
