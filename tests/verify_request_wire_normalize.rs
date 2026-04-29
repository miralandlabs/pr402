//! Regression: handler scheme slugs in verify payloads normalize to wire forms.

use pr402::proto::VerifyRequest;
use pr402::util::{
    HANDLER_SCHEME_EXACT, HANDLER_SCHEME_SLA_ESCROW, WIRE_SCHEME_EXACT, WIRE_SCHEME_SLA_ESCROW,
};
use serde_json::json;
use std::collections::HashMap;

fn sample_network() -> serde_json::Value {
    json!("solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1")
}

#[test]
fn normalize_verify_request_exact_handler_slug() {
    let mut req = VerifyRequest {
        x402_version: 2,
        payment_payload: json!({
            "x402Version": 2,
            "accepted": {
                "scheme": HANDLER_SCHEME_EXACT,
                "network": sample_network(),
                "amount": "1",
                "payTo": "Pay111111111111111111111111111111111111111",
                "maxTimeoutSeconds": 60,
                "asset": "11111111111111111111111111111111"
            },
            "payload": { "transaction": "AQ==" },
            "extensions": {}
        }),
        payment_requirements: json!({
            "scheme": HANDLER_SCHEME_EXACT,
            "network": sample_network(),
            "amount": "1",
            "payTo": "Pay111111111111111111111111111111111111111",
            "maxTimeoutSeconds": 60,
            "asset": "11111111111111111111111111111111"
        }),
        correlation_id: None,
        extra: HashMap::new(),
    };

    req.normalize_scheme_slugs_for_wire();

    assert_eq!(
        req.payment_requirements["scheme"].as_str(),
        Some(WIRE_SCHEME_EXACT)
    );
    assert_eq!(
        req.payment_payload["accepted"]["scheme"].as_str(),
        Some(WIRE_SCHEME_EXACT)
    );
}

#[test]
fn normalize_verify_request_sla_escrow_handler_slug() {
    let mut req = VerifyRequest {
        x402_version: 2,
        payment_payload: json!({
            "x402Version": 2,
            "accepted": {
                "scheme": HANDLER_SCHEME_SLA_ESCROW,
                "network": sample_network(),
                "amount": "1",
                "payTo": "Esc111111111111111111111111111111111111111",
                "maxTimeoutSeconds": 60,
                "asset": "11111111111111111111111111111111"
            },
            "payload": { "transaction": "AQ==" },
            "extensions": {}
        }),
        payment_requirements: json!({
            "scheme": HANDLER_SCHEME_SLA_ESCROW,
            "network": sample_network(),
            "amount": "1",
            "payTo": "Esc111111111111111111111111111111111111111",
            "maxTimeoutSeconds": 60,
            "asset": "11111111111111111111111111111111"
        }),
        correlation_id: None,
        extra: HashMap::new(),
    };

    req.normalize_scheme_slugs_for_wire();

    assert_eq!(
        req.payment_requirements["scheme"].as_str(),
        Some(WIRE_SCHEME_SLA_ESCROW)
    );
    assert_eq!(
        req.payment_payload["accepted"]["scheme"].as_str(),
        Some(WIRE_SCHEME_SLA_ESCROW)
    );
}

#[test]
fn normalize_skips_non_v2() {
    let mut req = VerifyRequest {
        x402_version: 1,
        payment_payload: json!({
            "accepted": { "scheme": HANDLER_SCHEME_EXACT }
        }),
        payment_requirements: json!({ "scheme": HANDLER_SCHEME_EXACT }),
        correlation_id: None,
        extra: HashMap::new(),
    };

    req.normalize_scheme_slugs_for_wire();

    assert_eq!(
        req.payment_requirements["scheme"].as_str(),
        Some(HANDLER_SCHEME_EXACT)
    );
}
