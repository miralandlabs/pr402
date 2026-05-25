use super::*;

pub async fn handle_capabilities(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
) -> Response<Body> {
    let supported = match facilitator.supported().await {
        Ok(s) => s,
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Error: {}", e))
        }
    };
    let supported_json = match serde_json::to_value(&supported) {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("serialization failed: {}", e),
            );
        }
    };

    let (chain_id, fee_payer, universal_settle, sla_escrow) = if let Some(cp) = CHAIN_PROVIDER.get()
    {
        (
            cp.solana.chain_id().to_string(),
            cp.solana.fee_payer().to_string(),
            cp.solana.universalsettle().is_some(),
            cp.solana.sla_escrow().is_some(),
        )
    } else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "chain provider not initialized",
        );
    };

    let sla_escrow_oracle_profiles = if sla_escrow {
        // Refresh the parameters cache so we read DB-backed overrides ahead of env vars.
        // The `parameters` table is the preferred source on Vercel (avoids the env size limit).
        pr402::parameters::refresh_parameters_from_db(pr402_db()).await;
        let mut profiles = build_sla_escrow_oracle_profiles();
        // Wave A §3.2 — annotate each profile with health status when the gate is on.
        // The probe is cached for 30s so back-to-back capability requests are cheap.
        if let Some(list) = profiles.as_mut() {
            for entry in list.iter_mut() {
                if let Some(err) =
                    pr402::oracle_health::probe_unhealthy(entry.registry_url.as_deref()).await
                {
                    entry.unhealthy = Some(true);
                    entry.last_health_error = Some(err);
                }
            }
        }
        profiles
    } else {
        None
    };

    let body = CapabilitiesResponse {
        schema_version: SCHEMA_VERSION,
        x402_version: 2,
        name: "pr402 facilitator",
        chain_id,
        fee_payer,
        supported: supported_json,
        features: CapabilitiesFeatures {
            universal_settle_exact: universal_settle,
            sla_escrow,
            unsigned_exact_payment_tx_build: true,
            unsigned_sla_escrow_payment_tx_build: sla_escrow,
            seller_lifecycle_block: true,
            accepts_base64_onboard_signature: true,
            build_response_signer_pubkeys: true,
            // Directory surface requires Postgres (lists read from `resource_providers`).
            public_provider_directory: pr402_db().is_some(),
        },
        http_endpoints: CapabilitiesHttpEndpoints {
            verify: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/verify",
                auth: None,
            },
            settle: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/settle",
                auth: None,
            },
            build_exact_payment_tx: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/build-exact-payment-tx",
                auth: None,
            },
            build_sla_escrow_payment_tx: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/build-sla-escrow-payment-tx",
                auth: None,
            },
            build_oracle_confirm_tx: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/oracle/build-confirm",
                auth: None,
            },
            build_refund_tx: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/build-refund-tx",
                auth: None,
            },
            sweep: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/sweep",
                auth: Some("bearer"),
            },
            sweep_cron: HttpEndpointInfo {
                method: "GET",
                path: "/api/v1/facilitator/sweep-cron",
                auth: Some("bearer"),
            },
            sla_escrow_settle: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/sla-escrow-settle",
                auth: Some("bearer"),
            },
            sla_escrow_settle_cron: HttpEndpointInfo {
                method: "GET",
                path: "/api/v1/facilitator/sla-escrow-settle-cron",
                auth: Some("bearer"),
            },
            onboard_preview: HttpEndpointInfo {
                method: "GET",
                path: "/api/v1/facilitator/onboard",
                auth: None,
            },
            onboard_challenge: HttpEndpointInfo {
                method: "GET",
                path: "/api/v1/facilitator/onboard/challenge",
                auth: None,
            },
            onboard: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/onboard",
                auth: None,
            },
            onboard_provision: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/onboard/provision",
                auth: None,
            },
            onboard_retire: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/onboard/retire",
                auth: None,
            },
            providers: HttpEndpointInfo {
                method: "GET",
                path: "/api/v1/facilitator/providers",
                auth: None,
            },
            provider: HttpEndpointInfo {
                method: "GET",
                path: "/api/v1/facilitator/providers/{wallet}",
                auth: None,
            },
            seller_payments: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/seller/payments",
                auth: None,
            },
            supported: HttpEndpointInfo {
                method: "GET",
                path: "/api/v1/facilitator/supported",
                auth: None,
            },
            health: HttpEndpointInfo {
                method: "GET",
                path: "/api/v1/facilitator/health",
                auth: None,
            },
            capabilities: HttpEndpointInfo {
                method: "GET",
                path: "/api/v1/facilitator/capabilities",
                auth: None,
            },
            discovery: HttpEndpointInfo {
                method: "GET",
                path: "/api/v1/facilitator/discovery",
                auth: None,
            },
            upgrade: HttpEndpointInfo {
                method: "POST",
                path: "/api/v1/facilitator/upgrade",
                auth: None,
            },
        },
        agent_manifest: AgentManifest {
            open_api: "/openapi.json",
            pay_to_semantics: "/agent-payTo-semantics.json",
            integration_guide: "/agent-integration.md",
            seller_quick_start: "/seller-quick-start.md",
            seller_onboarding_guide: "/onboarding_guide.md",
            buyer_quick_start: "/quickstart-buyer.md",
            x402_spec: "https://github.com/coinbase/x402/blob/main/specs/x402-specification-v2.md",
        },
        sla_escrow_oracle_profiles,
    };

    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(serde_json::to_string(&body).unwrap_or_default()))
        .unwrap()
}

/// Programmatic Discovery: Find payTo address and extra metadata.
/// Query: `wallet=<PUBKEY>&scheme=<SCHEME>&asset=<MINT>`.
pub async fn handle_supported(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
) -> Response<Body> {
    match facilitator.supported().await {
        Ok(response) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(serde_json::to_string(&response).unwrap_or_else(
                |_| r#"{"error":"serialization failed"}"#.to_string(),
            )))
            .unwrap(),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Error: {}", e)),
    }
}

/// URL for `@solana/web3.js` `Connection` in the browser. Public `api.mainnet-beta.solana.com`
/// often returns JSON-RPC **403** from browsers. Uses the same RPC as the facilitator
/// (`SOLANA_RPC_URL` / [`CHAIN_PROVIDER`]) when that URL is not localhost; otherwise the public
/// cluster default for the detected network.
fn solana_wallet_rpc_url_for_browser(network: &str) -> String {
    if let Some(cp) = CHAIN_PROVIDER.get() {
        let s = cp.solana.rpc_url();
        if !s.contains("127.0.0.1") && !s.contains("localhost") {
            return s.to_string();
        }
    }
    if network == "devnet" {
        "https://api.devnet.solana.com".to_string()
    } else {
        "https://api.mainnet-beta.solana.com".to_string()
    }
}

pub async fn handle_health() -> Response<Body> {
    let mut db_status = "disabled";
    if let Some(db) = pr402_db() {
        db_status = match db.ping().await {
            Ok(_) => "connected",
            Err(_) => "error",
        };
    }

    let mut rpc_status = "error";
    let mut slot = None;
    let mut environment = "Production".to_string();
    let mut network = "mainnet".to_string();

    if let Some(cp) = CHAIN_PROVIDER.get() {
        // DETECT ENVIRONMENT: Match 'devnet' for Preview.
        let rpc_url = cp.solana.rpc_url();
        if rpc_url.contains("devnet") {
            environment = "Preview".to_string();
            network = "devnet".to_string();
        }

        match cp.solana.get_health().await {
            Ok(s) => {
                rpc_status = "connected";
                slot = Some(s);
            }
            Err(_) => rpc_status = "error",
        }
    }

    let solana_wallet_rpc_url = solana_wallet_rpc_url_for_browser(&network);

    let vault_sweep_token = pr402::parameters::resolve_sweep_cron_token(pr402_db()).await;
    let sla_settle_token =
        pr402::parameters::resolve_sla_escrow_settle_cron_token(pr402_db()).await;

    let res = HealthResponse {
        status: if rpc_status == "connected" {
            "ok"
        } else {
            "warning"
        },
        schema_version: SCHEMA_VERSION,
        database: db_status,
        solana_rpc: rpc_status,
        solana_slot: slot,
        environment,
        solana_network: network,
        solana_wallet_rpc_url,
        settlement_keeper: SettlementKeeperHealth {
            vault_sweep_cron_configured: vault_sweep_token
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty()),
            sla_escrow_settle_cron_configured: sla_settle_token
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty()),
            database_connected: db_status == "connected",
        },
    };

    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(serde_json::to_string(&res).unwrap_or_default()))
        .unwrap()
}

/// Stable discovery document for agents / dashboards (machine-readable complement to `/supported`).
pub async fn handle_discovery(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
    query: &str,
) -> Response<Body> {
    let wallet = query_param(query, "wallet");
    let scheme = query_param(query, "scheme");
    let asset = query_param(query, "asset");
    let asset_opt = if asset.is_empty() {
        None
    } else {
        Some(asset.as_str())
    };

    if wallet.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "Missing wallet parameter");
    }
    if scheme.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Missing scheme parameter (e.g. exact or sla-escrow)",
        );
    }

    match facilitator.discovery(&wallet, &scheme, asset_opt).await {
        Ok(info) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(serde_json::to_string(&info).unwrap_or_else(
                |_| r#"{"error":"serialization failed"}"#.to_string(),
            )))
            .unwrap(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Discovery failed: {}", e),
        ),
    }
}

/// Public PDA preview only (no DB). Use challenge + POST `/onboard` to persist with proof-of-control.
pub async fn handle_upgrade(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
    body: Body,
) -> Response<Body> {
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };

    let upgrade_request: pr402::proto::PaymentRequired = match serde_json::from_str(&body_str) {
        Ok(req) => req,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, &format!("Invalid request: {}", e));
        }
    };

    match facilitator.upgrade(&upgrade_request).await {
        Ok(response) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(serde_json::to_string(&response).unwrap_or_else(
                |_| r#"{"error":"serialization failed"}"#.to_string(),
            )))
            .unwrap(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, &format!("Upgrade failed: {}", e)),
    }
}

/// Build the `slaEscrowOracleProfiles[]` list advertised on `GET /capabilities`.
///
/// **Configuration precedence** — every key below follows pr402's parameters-first
/// convention via [`pr402::parameters::resolve_string_sync`]: the DB `parameters` row
/// wins (preferred for Vercel deployments to avoid the env-size limit), else the
/// matching env var of the same name, else the per-key default.
///
/// **Two configuration modes** (mutually exclusive — JSON wins when set):
///
/// 1. **Full JSON override** —
///    [`pr402::parameters::PR402_SLA_ESCROW_ORACLE_PROFILES_JSON`] holds a JSON array
///    of objects with the same shape as the response. Highest flexibility (custom
///    profile ids, multiple operators per profile, custom registry URLs).
///
/// 2. **Ergonomic per-profile keys** — when the JSON key is unset, three groups of
///    four keys each (DEFAULT_PUBKEY / NORMATIVE_SPEC_URL / REGISTRY_URL /
///    EVIDENCE_REGISTRY_NOTE) drive a default-shaped entry per profile. Profiles
///    with no DEFAULT_PUBKEY are omitted (an entry with only metadata adds no
///    actionable value to buyers).
///
/// Returns `None` when neither mode produces any entry — `/capabilities` then omits
/// the field entirely (preserves field-presence semantics for integrators).
///
/// **Caller contract:** call [`pr402::parameters::refresh_parameters_from_db`] before
/// invoking this so the DB cache is warm.
fn build_sla_escrow_oracle_profiles() -> Option<Vec<SlaEscrowOracleProfileInfo>> {
    use pr402::parameters as p;

    // 1. JSON override.
    if let Some(raw) = p::resolve_string_sync(
        p::PR402_SLA_ESCROW_ORACLE_PROFILES_JSON,
        p::PR402_SLA_ESCROW_ORACLE_PROFILES_JSON,
    ) {
        match serde_json::from_str::<Vec<serde_json::Value>>(&raw) {
            Ok(parsed) => {
                let entries: Vec<SlaEscrowOracleProfileInfo> = parsed
                    .into_iter()
                    .filter_map(|v| {
                        Some(SlaEscrowOracleProfileInfo {
                            profile_id: v.get("profileId")?.as_str()?.to_string(),
                            normative_spec_url: v
                                .get("normativeSpecUrl")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string(),
                            repository_path: v
                                .get("repositoryPath")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string(),
                            default_operator_pubkey: v
                                .get("defaultOperatorPubkey")
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string())
                                .filter(|s| !s.is_empty()),
                            registry_url: v
                                .get("registryUrl")
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string())
                                .filter(|s| !s.is_empty()),
                            evidence_registry_note: v
                                .get("evidenceRegistryNote")
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string())
                                .filter(|s| !s.is_empty()),
                            unhealthy: None,
                            last_health_error: None,
                        })
                    })
                    .collect();
                if !entries.is_empty() {
                    return Some(entries);
                }
                // Empty/invalid JSON falls through to per-profile keys below.
            }
            Err(e) => {
                // Bad JSON: log and fall through. We don't fail capabilities just
                // because the operator's config is malformed — the per-profile path
                // is still tried.
                eprintln!(
                    "warning: PR402_SLA_ESCROW_ORACLE_PROFILES_JSON parse failed ({}); \
                     falling back to per-profile keys",
                    e
                );
            }
        }
    }

    // 2. Ergonomic per-profile keys (DB parameters row > env > default).
    let mut entries: Vec<SlaEscrowOracleProfileInfo> = Vec::new();

    // (profile_id, repo_path, default_spec_url, pubkey_key, spec_url_key, registry_url_key, evidence_note_key)
    let cfg: &[(&str, &str, &str, &str, &str, &str, &str)] = &[
        (
            "x402/oracles/api-quality/v1",
            "oracle-api-quality/spec/api-quality-v1/NORMATIVE.md",
            "https://github.com/miraland-labs/oracles/blob/main/oracle-api-quality/spec/api-quality-v1/NORMATIVE.md",
            p::PR402_SLA_ESCROW_API_QUALITY_DEFAULT_PUBKEY,
            p::PR402_SLA_ESCROW_API_QUALITY_NORMATIVE_SPEC_URL,
            p::PR402_SLA_ESCROW_API_QUALITY_REGISTRY_URL,
            p::PR402_SLA_ESCROW_API_QUALITY_EVIDENCE_REGISTRY_NOTE,
        ),
        (
            "x402/oracles/onchain-transfer/v1",
            "oracle-onchain-transfer/spec/onchain-transfer-v1/NORMATIVE.md",
            "https://github.com/miraland-labs/oracles/blob/main/oracle-onchain-transfer/spec/onchain-transfer-v1/NORMATIVE.md",
            p::PR402_SLA_ESCROW_ONCHAIN_TRANSFER_DEFAULT_PUBKEY,
            p::PR402_SLA_ESCROW_ONCHAIN_TRANSFER_NORMATIVE_SPEC_URL,
            p::PR402_SLA_ESCROW_ONCHAIN_TRANSFER_REGISTRY_URL,
            p::PR402_SLA_ESCROW_ONCHAIN_TRANSFER_EVIDENCE_REGISTRY_NOTE,
        ),
        (
            "x402/oracles/file-delivery/attestation/v1",
            "oracle-file-delivery/spec/file-delivery-attestation-v1/NORMATIVE.md",
            "https://github.com/miraland-labs/oracles/blob/main/oracle-file-delivery/spec/file-delivery-attestation-v1/NORMATIVE.md",
            p::PR402_SLA_ESCROW_FILE_DELIVERY_DEFAULT_PUBKEY,
            p::PR402_SLA_ESCROW_FILE_DELIVERY_NORMATIVE_SPEC_URL,
            p::PR402_SLA_ESCROW_FILE_DELIVERY_REGISTRY_URL,
            p::PR402_SLA_ESCROW_FILE_DELIVERY_EVIDENCE_REGISTRY_NOTE,
        ),
    ];

    for (
        profile_id,
        repo_path,
        default_spec_url,
        pubkey_key,
        spec_url_key,
        registry_url_key,
        evidence_note_key,
    ) in cfg
    {
        // Per-profile entries are only emitted when the operator wants to advertise a
        // default. A "the profile exists somewhere" entry without a pubkey adds no value.
        let default_pubkey = p::resolve_string_sync(pubkey_key, pubkey_key);
        if default_pubkey.is_none() {
            continue;
        }
        entries.push(SlaEscrowOracleProfileInfo {
            profile_id: (*profile_id).to_string(),
            normative_spec_url: p::resolve_string_sync(spec_url_key, spec_url_key)
                .unwrap_or_else(|| (*default_spec_url).to_string()),
            repository_path: (*repo_path).to_string(),
            default_operator_pubkey: default_pubkey,
            registry_url: p::resolve_string_sync(registry_url_key, registry_url_key),
            evidence_registry_note: p::resolve_string_sync(evidence_note_key, evidence_note_key),
            unhealthy: None,
            last_health_error: None,
        });
    }

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}
