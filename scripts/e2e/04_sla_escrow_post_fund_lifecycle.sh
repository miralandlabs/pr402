#!/usr/bin/env bash
#
# After B1/B2 fund + /verify + /settle: drive on-chain SLA steps and mirror into Postgres.
#
#   submit-delivery → confirm-oracle → release-payment   (default)
#   submit-delivery → confirm-oracle (reject) → refund-payment   (E2E_SLA_LIFECYCLE_REFUND=1)
#
# Requires:
#   - Built pr402 `record_escrow_lifecycle` and `sla-escrow` CLI (admin not needed for these subcommands).
#   - E2E_ORACLE_KEYPAIR whose pubkey matches `escrow_details.oracle_authority` for this payment
#     (use the same oracle as fund-time, e.g. dev stack you control — not the preview oracle unless you have its key).
#   - DATABASE_URL (or pr402/.env) for DB writes.
#
# Env:
#   E2E_LIFECYCLE_CORRELATION_ID — payment_uid / correlation_id from a completed SLA fund E2E (default: /tmp/pr402_e2e_last_sla_correlation_id)
#   E2E_ORACLE_KEYPAIR — oracle signer (required)
#   E2E_SELLER_KEYPAIR, E2E_BUYER_KEYPAIR — defaults in common.sh
#   E2E_SLA_LIFECYCLE_REFUND=1 — refund path (buyer signs refund after oracle reject)
#   E2E_SLA_LIFECYCLE_DELIVERY_HASH — 64 hex chars (default: deterministic devnet test hash)
#   RESOLUTION_APPROVED=1 or 2 — only for approve path default 1
#
set -euo pipefail
source "$(dirname "$0")/common.sh"

require_cmd solana
require_cmd psql

[[ -f "$SLA_ESCROW_CLI" ]] || {
  echo "❌ Build: (cd $WORKSPACE_ROOT/sla-escrow && cargo build -p sla-escrow-cli --features admin --release)"
  exit 1
}

# Prefer a prebuilt path when set; otherwise `cargo run` (works with custom CARGO_TARGET_DIR).
run_record() {
  if [[ -n "${RECORD_ESCROW_BIN:-}" && -x "${RECORD_ESCROW_BIN}" ]]; then
    DATABASE_URL="$DATABASE_URL" "${RECORD_ESCROW_BIN}" "$@"
  else
    (cd "$PR402_ROOT" && DATABASE_URL="$DATABASE_URL" cargo run --release --bin record_escrow_lifecycle -- "$@")
  fi
}

[[ -f "${E2E_ORACLE_KEYPAIR:?Set E2E_ORACLE_KEYPAIR to the oracle keypair used at fund time}" ]] || {
  echo "❌ E2E_ORACLE_KEYPAIR file not found"
  exit 1
}
[[ -f "$E2E_SELLER_KEYPAIR" ]] || {
  echo "❌ E2E_SELLER_KEYPAIR file not found"
  exit 1
}

CORRELATION_ID="${E2E_LIFECYCLE_CORRELATION_ID:-}"
if [[ -z "$CORRELATION_ID" && -f /tmp/pr402_e2e_last_sla_correlation_id ]]; then
  CORRELATION_ID="$(tr -d ' \n\r' </tmp/pr402_e2e_last_sla_correlation_id)"
fi
[[ -n "$CORRELATION_ID" ]] || {
  echo "❌ Set E2E_LIFECYCLE_CORRELATION_ID or run 01_sla_escrow_facilitator_verify.sh first (writes /tmp/pr402_e2e_last_sla_correlation_id)"
  exit 1
}

DELIVERY_HASH="${E2E_SLA_LIFECYCLE_DELIVERY_HASH:-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20}"
[[ ${#DELIVERY_HASH} -eq 64 ]] || {
  echo "❌ E2E_SLA_LIFECYCLE_DELIVERY_HASH must be 64 hex characters"
  exit 1
}

load_database_url_from_env_file
[[ -n "${DATABASE_URL:-}" ]] || {
  echo "❌ DATABASE_URL required (export or pr402/.env) to record lifecycle"
  exit 1
}

ORACLE_ADDR="$(solana address -k "$E2E_ORACLE_KEYPAIR")"
ORACLE_DB="$(
  psql "$DATABASE_URL" -t -A -v ON_ERROR_STOP=1 -c "
SELECT ed.oracle_authority FROM payment_attempts pa
JOIN escrow_details ed ON ed.payment_attempt_id = pa.id
WHERE pa.correlation_id = '${CORRELATION_ID//\'/\'\'}';
" 2>/dev/null || true
)"
[[ -n "$ORACLE_DB" ]] || {
  echo "❌ No escrow_details row for correlation_id=$CORRELATION_ID (complete fund + verify/settle first)"
  exit 1
}
if [[ "$ORACLE_DB" != "$ORACLE_ADDR" ]]; then
  echo "❌ Oracle keypair $ORACLE_ADDR does not match DB oracle_authority $ORACLE_DB"
  exit 1
fi

REFUND="${E2E_SLA_LIFECYCLE_REFUND:-0}"
RES_STATE="${RESOLUTION_APPROVED:-1}"
if [[ "$REFUND" == "1" ]]; then
  RES_STATE="2"
fi

echo "=============================================="
echo " E2E: SLA post-fund lifecycle (devnet)"
echo "  correlation_id=$CORRELATION_ID"
echo "  refund_path=$REFUND (resolution_state=$RES_STATE)"
echo "=============================================="

echo ""
echo ">>> [1/4] submit-delivery (seller)"
set +e
SUBMIT_OUT="$(
  "$SLA_ESCROW_CLI" submit-delivery \
    --rpc "$RPC_URL" \
    --keypair "$E2E_SELLER_KEYPAIR" \
    --priority-fee "${E2E_PRIORITY_FEE_MICROLAMPORTS:-1}" \
    --payment-uid "$CORRELATION_ID" \
    --mint "$E2E_USDC_MINT" \
    --delivery-hash "$DELIVERY_HASH" \
    --yes 2>&1
)"
SUBMIT_EC=$?
set -e
echo "$SUBMIT_OUT"
[[ "$SUBMIT_EC" -eq 0 ]] || exit "$SUBMIT_EC"

SUBMIT_SIG="$(parse_sla_escrow_tx_sig "$SUBMIT_OUT")"
[[ -n "$SUBMIT_SIG" ]] || {
  echo "❌ could not parse submit-delivery signature"
  exit 1
}
run_record submit-delivery \
  --correlation-id "$CORRELATION_ID" \
  --tx-signature "$SUBMIT_SIG" \
  --delivery-hash "$DELIVERY_HASH"

echo ""
echo ">>> [2/4] confirm-oracle (resolution_state=$RES_STATE)"
set +e
CONFIRM_OUT="$(
  "$SLA_ESCROW_CLI" confirm-oracle \
    --rpc "$RPC_URL" \
    --keypair "$E2E_ORACLE_KEYPAIR" \
    --priority-fee "${E2E_PRIORITY_FEE_MICROLAMPORTS:-1}" \
    --payment-uid "$CORRELATION_ID" \
    --mint "$E2E_USDC_MINT" \
    --delivery-hash "$DELIVERY_HASH" \
    --resolution-state "$RES_STATE" \
    --yes 2>&1
)"
CONFIRM_EC=$?
set -e
echo "$CONFIRM_OUT"
[[ "$CONFIRM_EC" -eq 0 ]] || exit "$CONFIRM_EC"

CONFIRM_SIG="$(parse_sla_escrow_tx_sig "$CONFIRM_OUT")"
[[ -n "$CONFIRM_SIG" ]] || {
  echo "❌ could not parse confirm-oracle signature"
  exit 1
}
run_record confirm-oracle \
  --correlation-id "$CORRELATION_ID" \
  --tx-signature "$CONFIRM_SIG" \
  --delivery-hash "$DELIVERY_HASH" \
  --resolution-state "$RES_STATE"

if [[ "$REFUND" == "1" ]]; then
  [[ -f "$E2E_BUYER_KEYPAIR" ]] || {
    echo "❌ refund path needs E2E_BUYER_KEYPAIR"
    exit 1
  }
  echo ""
  echo ">>> [3/4] refund-payment (buyer)"
  set +e
  REFUND_OUT="$(
    "$SLA_ESCROW_CLI" refund-payment \
      --rpc "$RPC_URL" \
      --keypair "$E2E_BUYER_KEYPAIR" \
      --priority-fee "${E2E_PRIORITY_FEE_MICROLAMPORTS:-1}" \
      --payment-uid "$CORRELATION_ID" \
      --mint "$E2E_USDC_MINT" \
      --yes 2>&1
  )"
  REFUND_EC=$?
  set -e
  echo "$REFUND_OUT"
  [[ "$REFUND_EC" -eq 0 ]] || exit "$REFUND_EC"
  REFUND_SIG="$(parse_sla_escrow_tx_sig "$REFUND_OUT")"
  [[ -n "$REFUND_SIG" ]] || {
    echo "❌ could not parse refund-payment signature"
    exit 1
  }
  run_record refund-payment \
    --correlation-id "$CORRELATION_ID" \
    --tx-signature "$REFUND_SIG"
else
  echo ""
  echo ">>> [3/4] release-payment (seller)"
  set +e
  RELEASE_OUT="$(
    "$SLA_ESCROW_CLI" release-payment \
      --rpc "$RPC_URL" \
      --keypair "$E2E_SELLER_KEYPAIR" \
      --priority-fee "${E2E_PRIORITY_FEE_MICROLAMPORTS:-1}" \
      --payment-uid "$CORRELATION_ID" \
      --mint "$E2E_USDC_MINT" \
      --yes 2>&1
  )"
  RELEASE_EC=$?
  set -e
  echo "$RELEASE_OUT"
  [[ "$RELEASE_EC" -eq 0 ]] || exit "$RELEASE_EC"
  RELEASE_SIG="$(parse_sla_escrow_tx_sig "$RELEASE_OUT")"
  [[ -n "$RELEASE_SIG" ]] || {
    echo "❌ could not parse release-payment signature"
    exit 1
  }
  run_record release-payment \
    --correlation-id "$CORRELATION_ID" \
    --tx-signature "$RELEASE_SIG"
fi

echo ""
echo ">>> [4/4] DB audit"
psql_audit_escrow_lifecycle "$CORRELATION_ID"

echo ""
echo "✅ SLA post-fund lifecycle + DB mirror finished."
