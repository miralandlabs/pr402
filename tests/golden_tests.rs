use pr402::proto::v2::BuildPaymentTxResponse;
use serde_json::json;

#[test]
fn test_build_payment_tx_response_golden() {
    let response = BuildPaymentTxResponse {
        x402_version: 2,
        transaction: "base64_encoded_transaction".to_string(),
        recent_blockhash: "blockhash123".to_string(),
        recent_blockhash_expires_at: 1234567890,
        fee_payer: "fee_payer_pubkey".to_string(),
        payer: "payer_pubkey".to_string(),
        payer_signature_index: 0,
        payment_uid: Some("uid_123".to_string()),
        verify_body_template: json!({ "template": "test" }),
        notes: vec!["Test note".to_string()],
    };

    let serialized = serde_json::to_value(&response).unwrap();

    let expected = json!({
        "x402Version": 2,
        "transaction": "base64_encoded_transaction",
        "recentBlockhash": "blockhash123",
        "recentBlockhashExpiresAt": 1234567890,
        "feePayer": "fee_payer_pubkey",
        "payer": "payer_pubkey",
        "payerSignatureIndex": 0,
        "paymentUid": "uid_123",
        "verifyBodyTemplate": {
            "template": "test"
        },
        "notes": ["Test note"]
    });

    assert_eq!(
        serialized, expected,
        "Serialized BuildPaymentTxResponse does not match golden expected JSON!"
    );
}

#[test]
fn test_build_payment_tx_response_omitted_optionals() {
    let response = BuildPaymentTxResponse {
        x402_version: 2,
        transaction: "base64_encoded_transaction".to_string(),
        recent_blockhash: "blockhash123".to_string(),
        recent_blockhash_expires_at: 1234567890,
        fee_payer: "fee_payer_pubkey".to_string(),
        payer: "payer_pubkey".to_string(),
        payer_signature_index: 1,
        payment_uid: None,
        verify_body_template: json!({ "template": "test" }),
        notes: vec![], // Empty vec shouldn't be omitted by default unless skip_serializing_if is used
    };

    let serialized = serde_json::to_value(&response).unwrap();

    // paymentUid should be omitted because of skip_serializing_if = "Option::is_none"
    assert!(serialized.get("paymentUid").is_none());
}
