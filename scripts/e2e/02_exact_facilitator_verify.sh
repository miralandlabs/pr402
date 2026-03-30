#!/usr/bin/env bash
#
# Scenario A: UniversalSettle (`exact`) — small devnet test amount (default 0.05 USDC raw = 50_000).
# Full x402 facilitator flow on devnet: build-exact-payment-tx → sign payer → POST /verify → POST /settle.
#
# Env:
#   E2E_SCENARIO_A_AMOUNT_RAW — raw USDC amount (6 decimals), default 50_000 (0.05 USDC)
#   E2E_EXACT_AMOUNT_RAW — if set, overrides scenario A amount (backward compat)
#   Same keypairs / RPC / FACILITATOR_URL as common.sh
#
set -euo pipefail
source "$(dirname "$0")/common.sh"

require_cmd solana
require_cmd curl
require_cmd jq
require_cmd python3

E2E_EXACT_AMOUNT_RAW="${E2E_EXACT_AMOUNT_RAW:-$E2E_SCENARIO_A_AMOUNT_RAW}"
PAYER_PK="$(buyer_pubkey)"
SELLER_PK="$(seller_pubkey)"

echo "=============================================="
echo " E2E Scenario A: exact (UniversalSettle, devnet test amount)"
echo "  → /verify + /settle (devnet)"
echo "=============================================="
echo " Policy (prod ref): < ${USDC_POLICY_THRESHOLD_WHOLE} USDC → exact — test raw amount below"
echo " Facilitator:   $FACILITATOR_URL"
echo " Amount raw:    $E2E_EXACT_AMOUNT_RAW (USDC smallest units)"
echo " Payer:         $PAYER_PK"
echo " PayTo:         $SELLER_PK"
echo ""

EXACT_EXTRA="$(curl -sS "$FACILITATOR_URL/api/v1/facilitator/supported" | jq '.kinds[] | select(.scheme=="exact") | .extra')"
[[ "$EXACT_EXTRA" != "null" && -n "$EXACT_EXTRA" ]] || {
  echo "❌ Facilitator /supported has no \"exact\" kind"
  exit 1
}

ACCEPTED="$(jq -n \
 --arg net "$DEVNET_CHAIN_ID" \
 --arg pay "$SELLER_PK" \
 --arg amt "$E2E_EXACT_AMOUNT_RAW" \
 --arg mint "$E2E_USDC_MINT" \
 --argjson x "$EXACT_EXTRA" \
 '{
   scheme: "exact",
   network: $net,
   payTo: $pay,
   amount: $amt,
   maxTimeoutSeconds: 3600,
   asset: $mint,
   extra: $x
 }')"

BUILD_BODY="$(jq -n \
  --arg payer "$PAYER_PK" \
  --argjson acc "$ACCEPTED" \
  '{
     payer: $payer,
     accepted: $acc,
     resource: { url: "https://e2e.pr402.local/devnet-exact", description: "", mimeType: "" },
     skipSourceBalanceCheck: true
   }')"

echo ">>> [1/4] POST build-exact-payment-tx"
BUILD_RES="$(curl -sS \
  -X POST "$FACILITATOR_URL/api/v1/facilitator/build-exact-payment-tx" \
  -H "Content-Type: application/json" \
  -d "$BUILD_BODY")"

echo "$BUILD_RES" | jq .

TX_UNSIGNED_B64="$(echo "$BUILD_RES" | jq -r '.transaction // empty')"
BLOCKHASH="$(echo "$BUILD_RES" | jq -r '.recentBlockhash // empty')"
[[ -n "$TX_UNSIGNED_B64" && -n "$BLOCKHASH" ]] || {
  echo "❌ build-exact failed or missing transaction / recentBlockhash"
  exit 1
}

echo ""
echo ">>> [2/4] Sign with cargo example e2e_sign_exact_tx"
cd "$PR402_ROOT"
SIGNED_B64="$(printf '%s' "$TX_UNSIGNED_B64" | cargo run -q --example e2e_sign_exact_tx -- "$E2E_BUYER_KEYPAIR" "$BLOCKHASH")"

VERIFY_TEMPLATE="$(echo "$BUILD_RES" | jq '.verifyBodyTemplate')"
VERIFY_BODY="$(echo "$VERIFY_TEMPLATE" | jq --arg t "$SIGNED_B64" '.paymentPayload.payload.transaction = $t')"

CORRELATION_ID="${E2E_CORRELATION_PREFIX:-e2e-exact}-$(date +%s)"
VERIFY_BODY="$(echo "$VERIFY_BODY" | jq --arg c "$CORRELATION_ID" '. + {correlationId: $c}')"

echo ""
echo ">>> [3/4] POST /verify"
HTTP_CODE="$(curl -sS -o /tmp/e2e_exact_verify.json -w "%{http_code}" \
  -X POST "$FACILITATOR_URL/api/v1/facilitator/verify" \
  -H "Content-Type: application/json" \
  -H "X-Correlation-Id: $CORRELATION_ID" \
  -d "$VERIFY_BODY")"

cat /tmp/e2e_exact_verify.json | jq .
echo "HTTP $HTTP_CODE"

if [[ "$HTTP_CODE" != "200" ]]; then
  echo "❌ exact verify HTTP $HTTP_CODE (funded payer ATA + vault for payTo may be required)"
  exit 1
fi

echo ""
echo ">>> [4/4] POST /settle (same body as verify)"
SETTLE_CODE="$(facilitator_settle "$VERIFY_BODY" "$CORRELATION_ID" /tmp/e2e_exact_settle.json)"
cat /tmp/e2e_exact_settle.json | jq .
echo "HTTP $SETTLE_CODE"

load_database_url_from_env_file
if [[ -n "${DATABASE_URL:-}" ]]; then
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c "
SELECT correlation_id, scheme, verify_ok, settle_ok, settlement_signature
FROM payment_attempts
WHERE correlation_id = '${CORRELATION_ID//\'/\'\'}';
"
fi

if [[ "$SETTLE_CODE" != "200" ]]; then
  echo "❌ exact settle HTTP $SETTLE_CODE"
  exit 1
fi

echo ""
echo "✅ Scenario A: UniversalSettle (exact) verify + settle E2E finished."
