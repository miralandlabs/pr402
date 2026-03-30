#!/usr/bin/env bash
#
# Devnet E2E: SLA-Escrow fund-payment (real tx) → POST /verify on pr402 → optional DB audit.
#
# Env:
#   FACILITATOR_URL, RPC_URL, E2E_USDC_MINT, E2E_BUYER_KEYPAIR, E2E_SELLER_KEYPAIR,
#   E2E_ADMIN_KEYPAIR, SLA_ESCROW_CLI, DATABASE_URL (optional; or read from pr402/.env)
#
set -euo pipefail
source "$(dirname "$0")/common.sh"

require_cmd solana
require_cmd curl
require_cmd jq
require_cmd python3

[[ -f "$SLA_ESCROW_CLI" ]] || {
  echo "❌ Build: (cd $WORKSPACE_ROOT/sla-escrow && cargo build -p sla-escrow-cli --features admin --release)"
  exit 1
}
[[ -f "$E2E_BUYER_KEYPAIR" && -f "$E2E_SELLER_KEYPAIR" ]] || {
  echo "❌ Need keypairs: E2E_BUYER_KEYPAIR E2E_SELLER_KEYPAIR"
  exit 1
}

SELLER="$(seller_pubkey)"
CORRELATION_ID="${E2E_CORRELATION_PREFIX:-e2e-sla}-$(date +%s)"
AMOUNT_HUMAN="${E2E_SLA_AMOUNT_HUMAN:-0.1}"
TTL_SEC="${E2E_SLA_TTL:-3600}"
SLA_HASH_ZERO="0000000000000000000000000000000000000000000000000000000000000000"

echo "=============================================="
echo " E2E: SLA-Escrow → pr402 /verify (devnet)"
echo "=============================================="
echo " RPC:             $RPC_URL"
echo " Facilitator:     $FACILITATOR_URL"
echo " Mint:            $E2E_USDC_MINT"
echo " Seller (payTo):  $SELLER"
echo " Buyer signs:     $(buyer_pubkey)"
echo " Correlation id:  $CORRELATION_ID"
echo ""

echo ">>> [1/4] open-escrow (USDC rail)"
"$SLA_ESCROW_SCRIPTS/open-escrow.sh" \
  --mint "$E2E_USDC_MINT" \
  --rpc "$RPC_URL" \
  --keypair "$E2E_ADMIN_KEYPAIR" \
  --yes || {
  echo "(open-escrow may already exist; continuing)"
}

echo ""
echo ">>> [2/4] fund-payment (onchain; captures AGENTIC_AUDIT_TX_B64)"
set +e
FUND_OUT=$(
  "$SLA_ESCROW_SCRIPTS/fund-payment.sh" \
    --seller "$SELLER" \
    --mint "$E2E_USDC_MINT" \
    --amount "$AMOUNT_HUMAN" \
    --amount-type human \
    --payment-uid "$CORRELATION_ID" \
    --oracle-authority "$SELLER" \
    --ttl-seconds "$TTL_SEC" \
    --sla-hash "$SLA_HASH_ZERO" \
    --rpc "$RPC_URL" \
    --keypair "$E2E_BUYER_KEYPAIR" \
    --yes 2>&1
)
FUND_EC=$?
set -e
echo "$FUND_OUT"
if [[ "$FUND_EC" -ne 0 ]]; then
  echo "❌ fund-payment failed (exit $FUND_EC)"
  exit "$FUND_EC"
fi

TX_B64="$(echo "$FUND_OUT" | grep "AGENTIC_AUDIT_TX_B64:" | sed 's/.*AGENTIC_AUDIT_TX_B64: //' | head -1 | tr -d '\r' | tr -d ' ')"
[[ -n "$TX_B64" ]] || {
  echo "❌ Could not parse AGENTIC_AUDIT_TX_B64 from fund-payment output"
  exit 1
}

RAW_MUL="1000000"
AMOUNT_RAW="$(python3 -c "import decimal; print(int(decimal.Decimal('$AMOUNT_HUMAN') * $RAW_MUL))")"

echo ""
echo ">>> [3/4] Fetch sla-escrow extra from facilitator /supported"
EXTRA_JSON="$(curl -sS "$FACILITATOR_URL/api/v1/facilitator/supported" | jq '.kinds[] | select(.scheme=="sla-escrow") | .extra')"

VERIFY_BODY="$(jq -n \
  --arg cid "$CORRELATION_ID" \
  --arg tx "$TX_B64" \
  --arg payto "$SELLER" \
  --arg amt "$AMOUNT_RAW" \
  --arg mint "$E2E_USDC_MINT" \
  --arg net "$DEVNET_CHAIN_ID" \
  --arg ttl "$TTL_SEC" \
  --argjson extra "$EXTRA_JSON" \
  '{
    x402Version: 2,
    correlationId: $cid,
    paymentRequirements: {
      scheme: "sla-escrow",
      network: $net,
      payTo: $payto,
      amount: $amt,
      maxTimeoutSeconds: ($ttl | tonumber),
      asset: $mint,
      extra: $extra
    },
    paymentPayload: {
      x402Version: 2,
      accepted: {
        scheme: "sla-escrow",
        network: $net,
        payTo: $payto,
        amount: $amt,
        maxTimeoutSeconds: ($ttl | tonumber),
        asset: $mint,
        extra: $extra
      },
      payload: { transaction: $tx }
    }
  }')"

echo ""
echo ">>> [4/4] POST /api/v1/facilitator/verify"
HTTP_CODE="$(curl -sS -o /tmp/e2e_verify_out.json -w "%{http_code}" \
  -X POST "$FACILITATOR_URL/api/v1/facilitator/verify" \
  -H "Content-Type: application/json" \
  -H "X-Correlation-Id: $CORRELATION_ID" \
  -d "$VERIFY_BODY")"

cat /tmp/e2e_verify_out.json | jq .
echo "HTTP $HTTP_CODE"

load_database_url_from_env_file
psql_audit_for_correlation "$CORRELATION_ID"

if [[ "$HTTP_CODE" != "200" ]]; then
  echo "❌ verify HTTP $HTTP_CODE"
  exit 1
fi

echo ""
echo "✅ SLA-Escrow facilitator verify E2E finished."
