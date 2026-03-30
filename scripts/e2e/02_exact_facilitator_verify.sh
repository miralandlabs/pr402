#!/usr/bin/env bash
#
# Devnet E2E: POST build-exact-payment-tx → sign payer (example) → POST /verify.
#
# Prereqs:
#   - Payer ATA funded for E2E_USDC_MINT (or expect simulation failure)
#   - Vault path: seller must have UniversalSettle vault (create-vault) when US config present
#
# Env: same as common.sh + E2E_EXACT_AMOUNT_RAW (default 100000 = 0.1 USDC)
#
set -euo pipefail
source "$(dirname "$0")/common.sh"

require_cmd solana
require_cmd curl
require_cmd jq
require_cmd python3

E2E_EXACT_AMOUNT_RAW="${E2E_EXACT_AMOUNT_RAW:-100000}"
PAYER_PK="$(buyer_pubkey)"
SELLER_PK="$(seller_pubkey)"

echo "=============================================="
echo " E2E: exact (UniversalSettle) → pr402 /verify"
echo "=============================================="
echo " Facilitator: $FACILITATOR_URL"
echo " Payer:       $PAYER_PK"
echo " PayTo:       $SELLER_PK"
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

echo ">>> [1/3] POST build-exact-payment-tx"
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
echo ">>> [2/3] Sign with cargo example e2e_sign_exact_tx"
cd "$PR402_ROOT"
SIGNED_B64="$(printf '%s' "$TX_UNSIGNED_B64" | cargo run -q --example e2e_sign_exact_tx -- "$E2E_BUYER_KEYPAIR" "$BLOCKHASH")"

VERIFY_TEMPLATE="$(echo "$BUILD_RES" | jq '.verifyBodyTemplate')"
VERIFY_BODY="$(echo "$VERIFY_TEMPLATE" | jq --arg t "$SIGNED_B64" '.paymentPayload.payload.transaction = $t')"

echo ""
echo ">>> [3/3] POST /verify"
CORRELATION_ID="${E2E_CORRELATION_PREFIX:-e2e-exact}-$(date +%s)"
VERIFY_BODY="$(echo "$VERIFY_BODY" | jq --arg c "$CORRELATION_ID" '. + {correlationId: $c}')"

HTTP_CODE="$(curl -sS -o /tmp/e2e_exact_verify.json -w "%{http_code}" \
  -X POST "$FACILITATOR_URL/api/v1/facilitator/verify" \
  -H "Content-Type: application/json" \
  -H "X-Correlation-Id: $CORRELATION_ID" \
  -d "$VERIFY_BODY")"

cat /tmp/e2e_exact_verify.json | jq .
echo "HTTP $HTTP_CODE"

load_database_url_from_env_file
if [[ -n "${DATABASE_URL:-}" ]]; then
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c "
SELECT correlation_id, scheme, verify_ok FROM payment_attempts
WHERE correlation_id = '${CORRELATION_ID//\'/\'\'}';
"
fi

if [[ "$HTTP_CODE" != "200" ]]; then
  echo "❌ exact verify HTTP $HTTP_CODE (funded payer ATA + vault for payTo may be required)"
  exit 1
fi

echo ""
echo "✅ Exact facilitator verify E2E finished."
