//! Optional facilitator-assigned ids for merging `/verify` and `/settle` in `payment_attempts`
//! when the client does not send `correlationId` (x402 does not require it).

/// New lexicographically sortable id (ULID) for [`super::db::Pr402Db::record_payment_verify`].
pub fn mint_correlation_id() -> String {
    ulid::Ulid::new().to_string()
}

/// Merge `correlationId` into a JSON object response (no-op if `json` is not an object).
pub fn merge_correlation_into_value(json: &mut serde_json::Value, correlation_id: &str) {
    if let Some(obj) = json.as_object_mut() {
        obj.insert(
            "correlationId".to_string(),
            serde_json::Value::String(correlation_id.to_string()),
        );
    }
}
