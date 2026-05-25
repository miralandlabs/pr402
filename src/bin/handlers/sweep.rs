use super::*;

use pr402::settlement_keeper::sources::CandidateSourceKind;
use pr402::settlement_keeper::{run_vault_sweep, VaultSweepConfig, VaultSweepRequest};

pub async fn handle_sweep(body: Body, authorization_header: Option<&str>) -> Response<Body> {
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => "{}".to_string(),
    };
    let req: SweepRequest = match serde_json::from_str(&body_str) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };
    execute_sweep(req, authorization_header).await
}

pub async fn handle_sweep_cron(authorization_header: Option<&str>) -> Response<Body> {
    execute_sweep(SweepRequest::default(), authorization_header).await
}

async fn execute_sweep(req: SweepRequest, authorization_header: Option<&str>) -> Response<Body> {
    if let Err(res) = authorize_sweep(authorization_header).await {
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

    let db = pr402_db();
    if db.is_none() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "DATABASE_URL must be configured for cron sweep candidate polling",
        );
    }

    pr402::parameters::refresh_parameters_from_db(db).await;

    let configured_limit = pr402::parameters::resolve_sweep_cron_batch_limit(
        db,
        pr402::parameters::DEFAULT_SWEEP_CRON_BATCH_LIMIT,
    )
    .await;
    let configured_cooldown = pr402::parameters::resolve_sweep_cron_cooldown_sec(
        db,
        pr402::parameters::DEFAULT_SWEEP_CRON_COOLDOWN_SEC,
    )
    .await;
    let configured_recent_window = pr402::parameters::resolve_sweep_cron_recent_settle_window_sec(
        db,
        pr402::parameters::DEFAULT_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC,
    )
    .await;

    let vault_req = VaultSweepRequest {
        limit: req.limit,
        cooldown_seconds: req.cooldown_seconds,
        require_recent_settle_within_seconds: req.require_recent_settle_within_seconds,
        dry_run: req.dry_run,
        candidate_source: None,
    };

    let outcome = match run_vault_sweep(
        vault_req,
        VaultSweepConfig {
            chain: cp,
            db,
            limit: configured_limit,
            cooldown_sec: configured_cooldown,
            recent_settle_window_sec: configured_recent_window,
            dry_run: req.dry_run.unwrap_or(false),
            candidate_source: CandidateSourceKind::Pr402Db,
        },
    )
    .await
    {
        Ok(o) => o,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };

    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(
            serde_json::to_string(&outcome).unwrap_or_default(),
        ))
        .unwrap()
}

pub async fn handle_vault_snapshot(query: &str) -> Response<Body> {
    let wallet = query_param(query, "wallet");
    let spl_mint = query_param(query, "spl_mint");
    let spl_token_program_raw = query_param(query, "spl_token_program");
    let spl_scope = query_param(query, "spl_balance_scope");
    if wallet.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "Missing wallet parameter");
    }
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Config error: {}", e),
            );
        }
    };
    let Some(us) = cfg.universalsettle.as_ref() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "UNIVERSALSETTLE_PROGRAM_ID not configured",
        );
    };
    let seller = match pr402::vault_balance::parse_seller(&wallet) {
        Ok(p) => p,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
    };
    let mint_opt = if spl_mint.is_empty() {
        None
    } else {
        match solana_pubkey::Pubkey::from_str(&spl_mint) {
            Ok(m) => Some(m),
            Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid spl_mint"),
        }
    };
    let token_prog_opt = if spl_token_program_raw.is_empty() {
        None
    } else {
        match solana_pubkey::Pubkey::from_str(&spl_token_program_raw) {
            Ok(p) => Some(p),
            Err(_) => {
                return error_response(StatusCode::BAD_REQUEST, "Invalid spl_token_program");
            }
        }
    };
    let rpc = solana_client::nonblocking::rpc_client::RpcClient::new_with_commitment(
        cfg.solana_rpc_url.to_string(),
        solana_commitment_config::CommitmentConfig::confirmed(),
    );
    let mut snap = match pr402::vault_balance::fetch_universalsettle_vault_snapshot(
        &rpc,
        us.program_id,
        seller,
        mint_opt,
        token_prog_opt,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
    };

    let owner_wallet_scope = spl_scope == "owner_wallet"
        || spl_scope == "owner_mint"
        || spl_scope == "get_token_accounts_by_owner";

    if owner_wallet_scope {
        if let Some(mint) = mint_opt {
            match pr402::vault_balance::fetch_spl_raw_balance_by_owner_and_mint(
                &rpc, &seller, &mint,
            )
            .await
            {
                Ok((raw, dec)) => {
                    snap.spl_amount_raw = raw;
                    snap.spl_decimals = dec.or(snap.spl_decimals);
                }
                Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
            }
        }
    }

    let spl_scope_out = if owner_wallet_scope && mint_opt.is_some() {
        "owner_wallet"
    } else {
        "vault_ata"
    };

    let body = serde_json::json!({
        "seller": snap.seller.to_string(),
        "programId": snap.program_id.to_string(),
        "splitVaultPda": snap.split_vault_pda.to_string(),
        "vaultSolStoragePda": snap.vault_sol_storage_pda.to_string(),
        "spendableLamports": snap.spendable_lamports,
        "vaultSplAta": snap.vault_spl_ata.map(|a| a.to_string()),
        "splAmountRaw": snap.spl_amount_raw,
        "splDecimals": snap.spl_decimals,
        "splBalanceScope": spl_scope_out,
    });
    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(body.to_string()))
        .unwrap()
}

fn parse_bearer_token(header: Option<&str>) -> Option<&str> {
    let raw = header?.trim();
    raw.strip_prefix("Bearer ").map(str::trim)
}

fn timing_safe_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

async fn authorize_sweep(header: Option<&str>) -> Result<(), Response<Body>> {
    let expected = pr402::parameters::resolve_sweep_cron_token(pr402_db())
        .await
        .filter(|s| !s.trim().is_empty());
    let Some(expected_token) = expected else {
        return Err(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_SWEEP_CRON_TOKEN not configured",
        ));
    };
    let Some(got_token) = parse_bearer_token(header) else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "Missing or invalid Authorization bearer token",
        ));
    };
    if !timing_safe_eq(got_token, &expected_token) {
        return Err(error_response(StatusCode::UNAUTHORIZED, "Unauthorized"));
    }
    Ok(())
}
