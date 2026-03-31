#!/usr/bin/env bash
#
# Scenario B2: SLA-Escrow with facilitator-paid Solana fees (Phase 5 default).
# Mirrors Scenario A: POST build-sla-escrow-payment-tx → sign buyer → /verify → /settle.
# Does NOT broadcast fund on-chain before verify (settle submits like `exact`).
#
# Env: same as common.sh + 01 (open-escrow still uses sla-escrow CLI admin).
#   E2E_SCENARIO_B_AMOUNT_HUMAN — default 1 USDC
#   E2E_SLA_AMOUNT_HUMAN — overrides amount if set
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

PAYER_PK="$(buyer_pubkey)"
SELLER="$(seller_pubkey)"
CORRELATION_ID="${E2E_CORRELATION_PREFIX:-e2e-sla-http}-$(date +%s)"
AMOUNT_HUMAN="${E2E_SLA_AMOUNT_HUMAN:-$E2E_SCENARIO_B_AMOUNT_HUMAN}"
TTL_SEC="${E2E_SLA_TTL:-3600}"
SLA_HASH_ZERO="0000000000000000000000000000000000000000000000000000000000000000"

echo ""
echo "################################################################################"
echo "# E2E START | B2 | SLA-Escrow | FACILITATOR pays Solana network fees (fee payer)"
echo "#           |    | HTTP: build-sla-escrow-payment-tx → buyer partial sign → /verify → /settle"
echo "#           |    | (Paired with B1: buyer-paid fees via CLI fund-payment.)"
echo "################################################################################"
echo ""
echo "=============================================="
echo " E2E Scenario B2: SLA-Escrow (HTTP build, facilitator fees)"
echo "  → build-sla-escrow-payment-tx → sign → /verify → /settle"
echo "=============================================="
echo " RPC:             $RPC_URL"
echo " Facilitator:     $FACILITATOR_URL"
echo " Mint:            $E2E_USDC_MINT"
echo " Amount (human):  $AMOUNT_HUMAN USDC"
echo " Seller (payTo):  $SELLER"
echo " Buyer (payer):   $PAYER_PK"
echo " paymentUid/corr: $CORRELATION_ID"
echo ""

echo ">>> [1/5] Fetch sla-escrow extra from /supported"
EXTRA_JSON="$(curl -sS "$FACILITATOR_URL/api/v1/facilitator/supported" | jq '.kinds[] | select(.scheme=="sla-escrow") | .extra')"
ORACLE_AUTH="${E2E_ORACLE_AUTHORITY:-$(echo "$EXTRA_JSON" | jq -r '.oracleAuthorities[0] // empty')}"
[[ -n "$ORACLE_AUTH" && "$ORACLE_AUTH" != "null" ]] || {
  echo "❌ No oracleAuthorities[0] from /supported"
  exit 1
}
echo " Oracle authority: $ORACLE_AUTH"

echo ""
echo ">>> [2/5] open-escrow (USDC rail)"
"$SLA_ESCROW_SCRIPTS/open-escrow.sh" \
  --mint "$E2E_USDC_MINT" \
  --rpc "$RPC_URL" \
  --keypair "$E2E_ADMIN_KEYPAIR" \
  --yes || {
  echo "(open-escrow may already exist; continuing)"
}

RAW_MUL="1000000"
AMOUNT_RAW="$(python3 -c "import decimal; print(int(decimal.Decimal('$AMOUNT_HUMAN') * $RAW_MUL))")"

ACCEPTED="$(jq -n \
  --arg net "$DEVNET_CHAIN_ID" \
  --arg pay "$SELLER" \
  --arg amt "$AMOUNT_RAW" \
  --arg mint "$E2E_USDC_MINT" \
  --arg ttl "$TTL_SEC" \
  --argjson extra "$EXTRA_JSON" \
  '{
    scheme: "sla-escrow",
    network: $net,
    payTo: $pay,
    amount: $amt,
    maxTimeoutSeconds: ($ttl | tonumber),
    asset: $mint,
    extra: $extra
  }')"

RESOURCE="$(jq -n \
  --arg url "https://e2e.pr402.local/devnet-sla-http" \
  '{ url: $url, description: "", mimeType: "" }')"

BUILD_BODY="$(jq -n \
  --arg payer "$PAYER_PK" \
  --argjson acc "$ACCEPTED" \
  --argjson res "$RESOURCE" \
  --arg oid "$CORRELATION_ID" \
  --arg ora "$ORACLE_AUTH" \
  --arg sla "$SLA_HASH_ZERO" \
  '{
    payer: $payer,
    accepted: $acc,
    resource: $res,
    slaHash: $sla,
    oracleAuthority: $ora,
    paymentUid: $oid,
    skipSourceBalanceCheck: true
  }')"

echo ""
echo ">>> [3/5] POST build-sla-escrow-payment-tx (default: facilitator network fee payer)"
BUILD_RES="$(curl -sS \
  -X POST "$FACILITATOR_URL/api/v1/facilitator/build-sla-escrow-payment-tx" \
  -H "Content-Type: application/json" \
  -d "$BUILD_BODY")"

echo "$BUILD_RES" | jq .

TX_UNSIGNED_B64="$(echo "$BUILD_RES" | jq -r '.transaction // empty')"
BLOCKHASH="$(echo "$BUILD_RES" | jq -r '.recentBlockhash // empty')"
FEE_PAYER_OUT="$(echo "$BUILD_RES" | jq -r '.feePayer // empty')"

[[ -n "$TX_UNSIGNED_B64" && -n "$BLOCKHASH" ]] || {
  echo "❌ build-sla-escrow-payment-tx failed or missing transaction / recentBlockhash"
  exit 1
}

echo ""
echo ">>> [4/5] Partial sign (buyer); facilitator completes at /settle"
cd "$PR402_ROOT"
SIGNED_B64="$(printf '%s' "$TX_UNSIGNED_B64" | cargo run -q --example e2e_sign_sla_escrow_tx -- "$E2E_BUYER_KEYPAIR" "$BLOCKHASH")"

VERIFY_TEMPLATE="$(echo "$BUILD_RES" | jq '.verifyBodyTemplate')"
VERIFY_BODY="$(echo "$VERIFY_TEMPLATE" | jq --arg t "$SIGNED_B64" '.paymentPayload.payload.transaction = $t')"
VERIFY_BODY="$(echo "$VERIFY_BODY" | jq --arg c "$CORRELATION_ID" '. + {correlationId: $c}')"

echo ""
echo ">>> [5a/5] POST /verify"
HTTP_CODE="$(curl -sS -o /tmp/e2e_sla_http_verify.json -w "%{http_code}" \
  -X POST "$FACILITATOR_URL/api/v1/facilitator/verify" \
  -H "Content-Type: application/json" \
  -H "X-Correlation-Id: $CORRELATION_ID" \
  -d "$VERIFY_BODY")"

cat /tmp/e2e_sla_http_verify.json | jq .
echo "HTTP $HTTP_CODE"

if [[ "$HTTP_CODE" != "200" ]]; then
  echo "❌ verify HTTP $HTTP_CODE (deployment must support facilitator fee payer SLA + build endpoint)"
  exit 1
fi

echo ""
echo ">>> [5b/5] POST /settle"
SETTLE_CODE="$(facilitator_settle "$VERIFY_BODY" "$CORRELATION_ID" /tmp/e2e_sla_http_settle.json)"
cat /tmp/e2e_sla_http_settle.json | jq .
echo "HTTP $SETTLE_CODE"

load_database_url_from_env_file
psql_audit_for_correlation "$CORRELATION_ID"

if [[ "$SETTLE_CODE" != "200" ]]; then
  echo "❌ settle HTTP $SETTLE_CODE"
  exit 1
fi

echo ""
echo "✅ Scenario B2: SLA-Escrow (facilitator fee payer via HTTP) verify + settle finished."
echo "   Response feePayer from build: $FEE_PAYER_OUT"
echo ""
echo "################################################################################"
echo "# E2E END   | B2 | SLA-Escrow | facilitator fee payer (HTTP) — completed OK"
echo "################################################################################"

echo "$CORRELATION_ID" >/tmp/pr402_e2e_last_sla_correlation_id
