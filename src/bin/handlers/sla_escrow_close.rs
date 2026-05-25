//! SLA-Escrow ClosePayment cron handler.

use super::*;

use pr402::settlement_keeper::sources::CandidateSourceKind;
use pr402::settlement_keeper::{run_sla_escrow_close, SlaEscrowCloseConfig, SlaEscrowCloseRequest};

pub async fn handle_sla_escrow_close(
    body: Body,
    authorization_header: Option<&str>,
) -> Response<Body> {
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => "{}".to_string(),
    };
    let req: SlaEscrowCloseRequest = match serde_json::from_str(&body_str) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };
    execute_sla_escrow_close(req, authorization_header).await
}

pub async fn handle_sla_escrow_close_cron(authorization_header: Option<&str>) -> Response<Body> {
    execute_sla_escrow_close(SlaEscrowCloseRequest::default(), authorization_header).await
}

async fn execute_sla_escrow_close(
    req: SlaEscrowCloseRequest,
    authorization_header: Option<&str>,
) -> Response<Body> {
    if let Err(res) = authorize_sla_escrow_settle(authorization_header).await {
        return res;
    }

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

    let configured_limit = pr402::parameters::resolve_sla_escrow_settle_cron_batch_limit(
        pr402_db(),
        pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_BATCH_LIMIT,
    )
    .await;
    let configured_deadline = pr402::parameters::resolve_sla_escrow_settle_cron_deadline_sec(
        pr402_db(),
        pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_DEADLINE_SEC,
    )
    .await;

    let outcome = match run_sla_escrow_close(
        req,
        SlaEscrowCloseConfig {
            chain: cp,
            program_id: escrow_config.program_id,
            limit: configured_limit,
            deadline_sec: configured_deadline,
            dry_run: false,
            candidate_source: CandidateSourceKind::ChainScan,
        },
    )
    .await
    {
        Ok(o) => o,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(
            serde_json::to_string(&outcome).unwrap_or_default(),
        ))
        .unwrap()
}

async fn authorize_sla_escrow_settle(header: Option<&str>) -> Result<(), Response<Body>> {
    let expected = pr402::parameters::resolve_sla_escrow_settle_cron_token(pr402_db())
        .await
        .filter(|s| !s.trim().is_empty());
    let Some(expected) = expected else {
        return Err(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_SLA_ESCROW_SETTLE_CRON_TOKEN not configured",
        ));
    };
    let Some(supplied) = header.and_then(|h| h.strip_prefix("Bearer ")) else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "missing or malformed bearer token",
        ));
    };
    if supplied.trim() != expected.trim() {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "invalid bearer token",
        ));
    }
    Ok(())
}
