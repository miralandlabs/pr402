use super::*;

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

/// Vercel cron helper (GET-compatible): runs sweep with configured defaults.
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
    let us_config = match cp.solana.universalsettle() {
        Some(us) => us,
        None => return error_response(StatusCode::BAD_REQUEST, "UniversalSettle not configured"),
    };
    let fee_destination = match us_config.fee_destination {
        Some(d) => d,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "UniversalSettle fee destination not configured",
            )
        }
    };

    let (cfg_pda, _) = cp.solana.get_config_pda(&us_config.program_id);
    let onchain_us: pr402::chain::solana_universalsettle::Config =
        match cp.solana.rpc_client().get_account(&cfg_pda).await {
            Ok(acc) => {
                let need = 8 + size_of::<pr402::chain::solana_universalsettle::Config>();
                if acc.data.len() < need {
                    return error_response(
                        StatusCode::BAD_GATEWAY,
                        "UniversalSettle config account data too small for current layout",
                    );
                }
                *bytemuck::from_bytes::<pr402::chain::solana_universalsettle::Config>(
                    &acc.data[8..need],
                )
            }
            Err(e) => {
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("failed to load UniversalSettle config: {}", e),
                );
            }
        };
    if onchain_us.fee_destination != fee_destination {
        warn!(
            target: LOG_SERVER_LOG,
            env_fee_dest = %fee_destination,
            chain_fee_dest = %onchain_us.fee_destination,
            "UniversalSettle cron sweep: configured fee_destination differs from on-chain treasury; using on-chain values for fee routing"
        );
    }

    let Some(db) = pr402_db() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "DATABASE_URL must be configured for cron sweep candidate polling",
        );
    };

    pr402::parameters::refresh_parameters_from_db(Some(db)).await;

    let configured_limit = pr402::parameters::resolve_sweep_cron_batch_limit(
        Some(db),
        pr402::parameters::DEFAULT_SWEEP_CRON_BATCH_LIMIT,
    )
    .await;
    let configured_cooldown = pr402::parameters::resolve_sweep_cron_cooldown_sec(
        Some(db),
        pr402::parameters::DEFAULT_SWEEP_CRON_COOLDOWN_SEC,
    )
    .await;
    let configured_recent_window = pr402::parameters::resolve_sweep_cron_recent_settle_window_sec(
        Some(db),
        pr402::parameters::DEFAULT_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC,
    )
    .await;

    let limit = req.limit.unwrap_or(configured_limit).clamp(1, 500);
    let cooldown_sec = req
        .cooldown_seconds
        .unwrap_or(configured_cooldown)
        .clamp(1, 86_400);
    let recent_window_sec = req
        .require_recent_settle_within_seconds
        .unwrap_or(configured_recent_window)
        .clamp(60, 7 * 86_400);
    let dry_run = req.dry_run.unwrap_or(false);

    let candidates = match db
        .list_sweep_candidates(cooldown_sec, recent_window_sec, limit)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("sweep candidate query failed: {}", e),
            )
        }
    };

    let mut attempted = 0u64;
    let mut succeeded = 0u64;
    let mut skipped_below_threshold = 0u64;
    let mut failed = 0u64;
    let mut items: Vec<SweepItemResult> = Vec::with_capacity(candidates.len());

    for c in candidates.iter() {
        let seller = match solana_pubkey::Pubkey::from_str(&c.wallet_pubkey) {
            Ok(pk) => pk,
            Err(_) => {
                failed += 1;
                items.push(SweepItemResult {
                    wallet: c.wallet_pubkey.clone(),
                    settlement_mode: c.settlement_mode.clone(),
                    spl_mint: c.spl_mint.clone(),
                    available_raw: 0,
                    threshold_raw: 0,
                    status: "failed".to_string(),
                    action: "none".to_string(),
                    signature: None,
                    error: Some("invalid wallet_pubkey in resource_providers".to_string()),
                });
                continue;
            }
        };

        let mint_opt = if c.settlement_mode == "spl" {
            match c.spl_mint.as_deref() {
                Some(m) => match solana_pubkey::Pubkey::from_str(m) {
                    Ok(pk) => Some(pk),
                    Err(_) => {
                        failed += 1;
                        items.push(SweepItemResult {
                            wallet: c.wallet_pubkey.clone(),
                            settlement_mode: c.settlement_mode.clone(),
                            spl_mint: c.spl_mint.clone(),
                            available_raw: 0,
                            threshold_raw: 0,
                            status: "failed".to_string(),
                            action: "none".to_string(),
                            signature: None,
                            error: Some("invalid spl_mint in resource_providers".to_string()),
                        });
                        continue;
                    }
                },
                None => {
                    failed += 1;
                    items.push(SweepItemResult {
                        wallet: c.wallet_pubkey.clone(),
                        settlement_mode: c.settlement_mode.clone(),
                        spl_mint: c.spl_mint.clone(),
                        available_raw: 0,
                        threshold_raw: 0,
                        status: "failed".to_string(),
                        action: "none".to_string(),
                        signature: None,
                        error: Some("spl settlement_mode without spl_mint".to_string()),
                    });
                    continue;
                }
            }
        } else {
            None
        };

        let token_program_opt = if let Some(mint) = mint_opt {
            match cp.solana.rpc_client().get_account(&mint).await {
                Ok(mint_acc) if mint_acc.owner == pr402::chain::solana::TOKEN_2022_PROGRAM_ID => {
                    Some(pr402::chain::solana::TOKEN_2022_PROGRAM_ID)
                }
                Ok(_) => Some(pr402::chain::solana::TOKEN_PROGRAM_ID),
                Err(_) => Some(pr402::chain::solana::TOKEN_PROGRAM_ID),
            }
        } else {
            None
        };

        let snap = match pr402::vault_balance::fetch_universalsettle_vault_snapshot(
            cp.solana.rpc_client(),
            us_config.program_id,
            seller,
            mint_opt,
            token_program_opt,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                failed += 1;
                items.push(SweepItemResult {
                    wallet: c.wallet_pubkey.clone(),
                    settlement_mode: c.settlement_mode.clone(),
                    spl_mint: c.spl_mint.clone(),
                    available_raw: 0,
                    threshold_raw: 0,
                    status: "failed".to_string(),
                    action: "none".to_string(),
                    signature: None,
                    error: Some(format!("vault snapshot failed: {}", e)),
                });
                continue;
            }
        };

        let provider_floor = c.sweep_threshold.unwrap_or(0);
        let global_floor = if let Some(mint) = mint_opt {
            pr402::parameters::resolve_sweep_min_spl_raw_for_mint(&mint)
        } else {
            pr402::parameters::resolve_u64_sync(
                pr402::parameters::PR402_SWEEP_MIN_SPENDABLE_LAMPORTS,
                pr402::parameters::PR402_SWEEP_MIN_SPENDABLE_LAMPORTS,
                pr402::parameters::DEFAULT_SWEEP_MIN_SPENDABLE_LAMPORTS,
            )
        };
        let threshold = std::cmp::max(provider_floor, global_floor);
        let available = if mint_opt.is_some() {
            snap.spl_amount_raw
        } else {
            snap.spendable_lamports
        };

        if available < threshold {
            skipped_below_threshold += 1;
            items.push(SweepItemResult {
                wallet: c.wallet_pubkey.clone(),
                settlement_mode: c.settlement_mode.clone(),
                spl_mint: c.spl_mint.clone(),
                available_raw: available,
                threshold_raw: threshold,
                status: "skipped".to_string(),
                action: "below_threshold".to_string(),
                signature: None,
                error: None,
            });
            continue;
        }

        if dry_run {
            items.push(SweepItemResult {
                wallet: c.wallet_pubkey.clone(),
                settlement_mode: c.settlement_mode.clone(),
                spl_mint: c.spl_mint.clone(),
                available_raw: available,
                threshold_raw: threshold,
                status: "eligible".to_string(),
                action: "dry_run".to_string(),
                signature: None,
                error: None,
            });
            continue;
        }

        attempted += 1;
        let is_sol = mint_opt.is_none();
        let token_mint = mint_opt.unwrap_or_default();
        let (fee_sol_recv, fee_token_owner) =
            pr402::chain::solana_universalsettle::sweep_fee_destinations(
                &us_config.program_id,
                &onchain_us.fee_destination,
                &seller,
                onchain_us.use_fee_shard,
                onchain_us.shard_count,
            );
        // Full sweep at execution time (`0`); threshold eligibility used `available` from snapshot above.
        let ix = pr402::chain::solana_universalsettle::build_sweep_instruction(
            us_config.program_id,
            cp.solana.pubkey(),
            snap.split_vault_pda,
            seller,
            fee_sol_recv,
            fee_token_owner,
            token_mint,
            0,
            is_sol,
            token_program_opt,
        );

        let send_result = async {
            let (recent_blockhash, _) = cp
                .solana
                .rpc_client()
                .get_latest_blockhash_with_commitment(
                    solana_commitment_config::CommitmentConfig::confirmed(),
                )
                .await
                .map_err(|e| e.to_string())?;
            let sweep_tx = solana_transaction::versioned::VersionedTransaction::from(
                solana_transaction::Transaction::new_signed_with_payer(
                    &[ix],
                    Some(&cp.solana.pubkey()),
                    &[cp.solana.keypair()],
                    recent_blockhash,
                ),
            );
            cp.solana
                .send_sweep_transaction(&sweep_tx)
                .await
                .map_err(|e| e.to_string())
        }
        .await;

        match send_result {
            Ok(sig) => {
                succeeded += 1;
                if let Err(e) = db
                    .record_sweep_attempt(
                        &c.wallet_pubkey,
                        &c.settlement_mode,
                        c.spl_mint.as_deref(),
                        Some(&sig.to_string()),
                    )
                    .await
                {
                    warn!(target: LOG_SERVER_LOG, error = %e, "record_sweep_attempt (success) failed");
                }
                items.push(SweepItemResult {
                    wallet: c.wallet_pubkey.clone(),
                    settlement_mode: c.settlement_mode.clone(),
                    spl_mint: c.spl_mint.clone(),
                    available_raw: available,
                    threshold_raw: threshold,
                    status: "ok".to_string(),
                    action: "sweep_submitted".to_string(),
                    signature: Some(sig.to_string()),
                    error: None,
                });
            }
            Err(e) => {
                failed += 1;
                if let Err(db_err) = db
                    .record_sweep_attempt(
                        &c.wallet_pubkey,
                        &c.settlement_mode,
                        c.spl_mint.as_deref(),
                        None,
                    )
                    .await
                {
                    warn!(target: LOG_SERVER_LOG, error = %db_err, "record_sweep_attempt (failure) failed");
                }
                items.push(SweepItemResult {
                    wallet: c.wallet_pubkey.clone(),
                    settlement_mode: c.settlement_mode.clone(),
                    spl_mint: c.spl_mint.clone(),
                    available_raw: available,
                    threshold_raw: threshold,
                    status: "failed".to_string(),
                    action: "sweep_error".to_string(),
                    signature: None,
                    error: Some(e),
                });
            }
        }
    }

    let body = serde_json::json!({
        "dryRun": dry_run,
        "scanned": candidates.len(),
        "attempted": attempted,
        "succeeded": succeeded,
        "skippedBelowThreshold": skipped_below_threshold,
        "failed": failed,
        "items": items
    });
    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(body.to_string()))
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
    let rpc =
        solana_client::nonblocking::rpc_client::RpcClient::new(cfg.solana_rpc_url.to_string());
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
