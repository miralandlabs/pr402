use super::*;

pub async fn handle_settle(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
    body: Body,
    correlation_http: Option<&str>,
) -> Response<Body> {
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };

    let settle_request: pr402::proto::SettleRequest = match serde_json::from_str(&body_str) {
        Ok(req) => req,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, &format!("Invalid request: {}", e));
        }
    };

    let persist_meta = settle_request.correlation_id_for_persistence(correlation_http);
    let (payee_wallet_opt, scheme_opt, amount_opt, asset_opt) = settle_request.v2_metadata();
    let backup_payee = settle_request.payee_wallet();
    let payee = payee_wallet_opt.as_deref().or(backup_payee.as_deref());
    let (settlement_mode, spl_mint_owned) = settle_request.resource_provider_settlement();
    let spl_mint_ref = spl_mint_owned.as_deref();

    pr402::parameters::refresh_parameters_from_db(pr402_db()).await;

    match facilitator.settle(&settle_request).await {
        Ok(response) => {
            let mut json = response.into_json();
            if let Some(cid) = persist_meta.as_deref() {
                pr402::payment_attempt::merge_correlation_into_value(&mut json, cid);
            }
            let sig = json
                .get("transaction")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if let (Some(db), Some(cid), Some(wallet)) =
                (pr402_db(), persist_meta.as_deref(), payee)
            {
                match db
                    .record_payment_settle(
                        cid,
                        ResourceProviderInfo {
                            wallet_pubkey: wallet,
                            rail: ResourceProviderRail {
                                settlement_mode: settlement_mode.as_str(),
                                spl_mint: spl_mint_ref,
                            },
                        },
                        PaymentOutcome {
                            ok: true,
                            error: None,
                            signature: sig.as_deref(),
                        },
                        PaymentAuditMetadata {
                            payer_wallet: None,
                            scheme: scheme_opt.as_deref(),
                            amount: amount_opt.as_deref(),
                            asset: asset_opt.as_deref(),
                        },
                    )
                    .await
                {
                    Ok(()) => {
                        persist_escrow_audit_if_applicable_settle(
                            db,
                            &settle_request,
                            cid,
                            scheme_opt.as_deref(),
                            sig.as_deref(),
                        )
                        .await;
                    }
                    Err(e) => {
                        warn!(
                            target: LOG_SERVER_LOG,
                            error = %e,
                            "record_payment_settle skipped"
                        );
                    }
                }
            }
            if let Some(cid) = persist_meta.as_deref() {
                info!(
                    target: LOG_SERVER_LOG,
                    correlation_id = %cid,
                    payee = %payee.unwrap_or("(none)"),
                    settlement_signature = sig.as_deref(),
                    "settle ok"
                );
            } else {
                info!(
                    target: LOG_SERVER_LOG,
                    payee = %payee.unwrap_or("(none)"),
                    db_enabled = pr402_db().is_some(),
                    "settle ok (no correlation id; payment still settled on-chain)"
                );
            }
            let mut res = facilitator_response!()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json");
            if let Some(cid) = persist_meta.as_deref() {
                res = res.header("X-Correlation-ID", cid);
            }
            res.body(Body::Text(serde_json::to_string(&json).unwrap_or_else(
                |_| r#"{"error":"serialization failed"}"#.to_string(),
            )))
            .unwrap()
        }
        Err(e) => {
            if let (Some(db), Some(cid), Some(wallet)) =
                (pr402_db(), persist_meta.as_deref(), payee)
            {
                let msg = format!("{}", e);
                if let Err(err) = db
                    .record_payment_settle(
                        cid,
                        ResourceProviderInfo {
                            wallet_pubkey: wallet,
                            rail: ResourceProviderRail {
                                settlement_mode: settlement_mode.as_str(),
                                spl_mint: spl_mint_ref,
                            },
                        },
                        PaymentOutcome {
                            ok: false,
                            error: Some(&msg),
                            signature: None,
                        },
                        PaymentAuditMetadata {
                            payer_wallet: None,
                            scheme: scheme_opt.as_deref(),
                            amount: amount_opt.as_deref(),
                            asset: asset_opt.as_deref(),
                        },
                    )
                    .await
                {
                    warn!(
                        target: LOG_SERVER_LOG,
                        error = %err,
                        "record_payment_settle skipped"
                    );
                }
            }
            error_response_with_optional_correlation(
                StatusCode::BAD_REQUEST,
                &format!("Settlement failed: {}", e),
                None,
                persist_meta.as_deref(),
            )
        }
    }
}
