//! SLA-Escrow settlement cron handler (thin wrapper over `settlement_keeper`).

use super::*;

use pr402::settlement_keeper::sources::CandidateSourceKind;
use pr402::settlement_keeper::{
    run_sla_escrow_settle, SlaEscrowSettleConfig, SlaEscrowSettleRequest,
};

pub async fn handle_sla_escrow_settle(
    body: Body,
    authorization_header: Option<&str>,
) -> Response<Body> {
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => "{}".to_string(),
    };
    let req: SlaEscrowSettleRequest = match serde_json::from_str(&body_str) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };
    execute_sla_escrow_settle(req, authorization_header).await
}

pub async fn handle_sla_escrow_settle_cron(authorization_header: Option<&str>) -> Response<Body> {
    execute_sla_escrow_settle(SlaEscrowSettleRequest::default(), authorization_header).await
}

async fn execute_sla_escrow_settle(
    req: SlaEscrowSettleRequest,
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

    let db = pr402_db();
    let needs_db =
        CandidateSourceKind::parse(req.candidate_source.as_deref()) == CandidateSourceKind::Pr402Db;
    if needs_db && db.is_none() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "DATABASE_URL must be configured for pr402_db candidate source",
        );
    }

    if let Some(db) = db {
        pr402::parameters::refresh_parameters_from_db(Some(db)).await;
    }

    let configured_limit = pr402::parameters::resolve_sla_escrow_settle_cron_batch_limit(
        db,
        pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_BATCH_LIMIT,
    )
    .await;
    let configured_cooldown = pr402::parameters::resolve_sla_escrow_settle_cron_cooldown_sec(
        db,
        pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_COOLDOWN_SEC,
    )
    .await;
    let configured_deadline = pr402::parameters::resolve_sla_escrow_settle_cron_deadline_sec(
        db,
        pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_DEADLINE_SEC,
    )
    .await;
    let configured_lookback = pr402::parameters::resolve_sla_escrow_settle_cron_lookback_sec(
        db,
        pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_LOOKBACK_SEC,
    )
    .await;

    let source_kind = CandidateSourceKind::parse(req.candidate_source.as_deref());

    let outcome = match run_sla_escrow_settle(
        req,
        SlaEscrowSettleConfig {
            chain: cp,
            db,
            program_id: escrow_config.program_id,
            limit: configured_limit,
            cooldown_sec: configured_cooldown,
            deadline_sec: configured_deadline,
            lookback_sec: configured_lookback,
            dry_run: false,
            candidate_source: source_kind,
        },
    )
    .await
    {
        Ok(o) => o,
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e);
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(
            serde_json::to_string(&outcome).unwrap_or_default(),
        ))
        .unwrap()
}

// Response-as-Err is the early-return pattern for these cron auth helpers.
#[allow(clippy::result_large_err)]
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
