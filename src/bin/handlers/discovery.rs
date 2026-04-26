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
