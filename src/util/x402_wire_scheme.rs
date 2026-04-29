//! x402 **verify/settle wire** scheme strings vs facilitator **handler** ids.
//!
//! Discovery, `402 accepts[]`, and some HTTP builders historically used namespaced
//! handler slugs (`v2:solana:exact`, `v2:solana:sla-escrow`). Typed `/verify` and
//! `/settle` paths deserialize [`ExactScheme`](crate::scheme::v2_solana_exact::types::ExactScheme)
//! and [`SLAEscrowScheme`](crate::scheme::v2_solana_escrow::types::SLAEscrowScheme), which only
//! accept **`exact`** and **`sla-escrow`** respectively.

/// Wire `PaymentRequirements.scheme` / `accepted.scheme` for the UniversalSettle exact rail.
pub const WIRE_SCHEME_EXACT: &str = "exact";
/// Wire scheme for the SLA escrow fund leg.
pub const WIRE_SCHEME_SLA_ESCROW: &str = "sla-escrow";

/// Handler-style id registered on [`crate::facilitator::FacilitatorLocal`].
pub const HANDLER_SCHEME_EXACT: &str = "v2:solana:exact";
/// Handler-style id for SLA escrow.
pub const HANDLER_SCHEME_SLA_ESCROW: &str = "v2:solana:sla-escrow";

/// Maps namespaced handler ids to x402 wire scheme strings for verify/settle JSON.
#[must_use]
pub fn to_wire_scheme(scheme: &str) -> &str {
    match scheme {
        HANDLER_SCHEME_EXACT => WIRE_SCHEME_EXACT,
        HANDLER_SCHEME_SLA_ESCROW => WIRE_SCHEME_SLA_ESCROW,
        other => other,
    }
}

/// If `value` is a JSON object with a string `scheme`, rewrite known handler ids to wire ids.
pub fn normalize_scheme_field_in_map(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let Some(sch) = obj.get_mut("scheme") else {
        return;
    };
    let Some(s) = sch.as_str() else {
        return;
    };
    let wired = to_wire_scheme(s);
    if wired != s {
        *sch = serde_json::Value::String(wired.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn to_wire_scheme_maps_handler_ids() {
        assert_eq!(to_wire_scheme(HANDLER_SCHEME_EXACT), WIRE_SCHEME_EXACT);
        assert_eq!(
            to_wire_scheme(HANDLER_SCHEME_SLA_ESCROW),
            WIRE_SCHEME_SLA_ESCROW
        );
        assert_eq!(to_wire_scheme(WIRE_SCHEME_EXACT), WIRE_SCHEME_EXACT);
        assert_eq!(
            to_wire_scheme(WIRE_SCHEME_SLA_ESCROW),
            WIRE_SCHEME_SLA_ESCROW
        );
        assert_eq!(to_wire_scheme("custom"), "custom");
    }

    #[test]
    fn normalize_scheme_field_in_map_mutates_handler_slugs_only() {
        let mut v = json!({"scheme": HANDLER_SCHEME_EXACT, "payTo": "x"});
        normalize_scheme_field_in_map(&mut v);
        assert_eq!(v["scheme"], WIRE_SCHEME_EXACT);

        let mut v2 = json!({"scheme": HANDLER_SCHEME_SLA_ESCROW});
        normalize_scheme_field_in_map(&mut v2);
        assert_eq!(v2["scheme"], WIRE_SCHEME_SLA_ESCROW);

        let mut noop = json!({"scheme": WIRE_SCHEME_EXACT});
        normalize_scheme_field_in_map(&mut noop);
        assert_eq!(noop["scheme"], WIRE_SCHEME_EXACT);

        let mut no_scheme = json!({"payTo": "y"});
        normalize_scheme_field_in_map(&mut no_scheme);
        assert_eq!(no_scheme, json!({"payTo": "y"}));
    }
}
