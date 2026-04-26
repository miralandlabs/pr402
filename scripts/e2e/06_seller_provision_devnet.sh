#!/usr/bin/env bash
#
# Seller provisioning E2E: POST /api/v1/facilitator/onboard/provision (UniversalSettle)
#
# Exercises:
#   A — New wallet, default USDC: expect unsigned tx → sign as seller → send; on-chain:
#       SplitVault PDA, SOL storage PDA, USDC vault ATA (three accounts).
#   A2 — Same wallet, different asset (WSOL), after DB registry row for USDC:
#       expect HTTP 400 from facilitator single-rail policy (needs Postgres + onboard).
#   B — New wallet, WSOL preset: same three-account pattern (vault + sol storage + WSOL ATA).
#   C — New wallet, custom devnet SPL mint: vault + sol storage + ATA for that mint.
#
# Env (see common.sh):
#   FACILITATOR_URL, RPC_URL / SOLANA_RPC_URL, DEVNET_CHAIN_ID
#   DEMO_FUNDER_KEYPAIR — funds fresh wallets (default: x402/demo-wallets/buyer-keypair.json;
#                         can use demo-wallets/seller-keypair.json if it holds devnet SOL)
#   E2E_PROVISION_FUND_SOL — SOL sent to each new wallet (default 0.5)
#   SKIP_SELLER_PROVISION_DB_POLICY — if 1, skip step A2 (onboard + second-asset 400 check)
#   E2E_CUSTOM_DEVNET_MINT — override mint for scenario C (default: devnet test mint in script)
#
# Prerequisites: solana, curl, jq, cargo (for examples/e2e_sign_exact_tx), spl-token optional
#
set -euo pipefail
source "$(dirname "$0")/common.sh"

# Submit + confirm using the same RPC the facilitator used for `recentBlockhash` (avoid fork / lag vs public devnet).
if [[ "${SYNC_RPC_FROM_FACILITATOR_HEALTH:-1}" == "1" ]]; then
  _hrpc="$(curl -sS "${FACILITATOR_URL}/api/v1/facilitator/health" | jq -r '.solanaWalletRpcUrl // empty')"
  if [[ -n "$_hrpc" && "$_hrpc" != "null" ]]; then
    export RPC_URL="$_hrpc"
    echo ">>> Using facilitator health RPC for submit/confirm: ${RPC_URL%%\?*}…"
  fi
fi

require_cmd solana
require_cmd curl
require_cmd jq
require_cmd cargo

E2E_PROVISION_FUND_SOL="${E2E_PROVISION_FUND_SOL:-0.5}"
E2E_CUSTOM_DEVNET_MINT="${E2E_CUSTOM_DEVNET_MINT:-2gNCDGj8Xi9Zs7LNQTPWf4pfZvAM7UHusY4xhKNYg6W6}"
SKIP_SELLER_PROVISION_DB_POLICY="${SKIP_SELLER_PROVISION_DB_POLICY:-0}"

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/pr402-provision-XXXXXX")"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

send_versioned_tx_b64() {
  local b64="$1" out err sig
  out="$(curl -sS "$RPC_URL" -X POST -H "Content-Type: application/json" -d "$(jq -n \
    --arg b "$b64" \
    '{jsonrpc:"2.0", id:1, method:"sendTransaction", params:[$b, {encoding:"base64", skipPreflight:false, maxRetries:5}]}')")"
  err="$(echo "$out" | jq -r '.error.message // empty')"
  if [[ -n "$err" ]]; then
    echo "$out" >&2
    return 1
  fi
  sig="$(echo "$out" | jq -r '.result')"
  printf '%s' "$sig"
}

assert_account() {
  local label="$1" pk="$2"
  if solana account "$pk" -u "$RPC_URL" >/dev/null 2>&1; then
    echo "  ✓ on-chain: $label"
  else
    echo "❌ missing account $label: $pk"
    return 1
  fi
}

new_seller_keypair() {
  local f="$1"
  # Silence keygen stdout/stderr so command substitution only captures `solana address`.
  solana-keygen new --no-bip39-passphrase --silent --force -o "$f" >/dev/null 2>&1
  solana address -k "$f"
}

fund_seller() {
  local addr="$1"
  echo ">>> Fund $addr with ${E2E_PROVISION_FUND_SOL} SOL from demo funder"
  [[ -f "$DEMO_FUNDER_KEYPAIR" ]] || {
    echo "❌ DEMO_FUNDER_KEYPAIR not found: $DEMO_FUNDER_KEYPAIR"
    exit 1
  }
  solana transfer "$addr" "$E2E_PROVISION_FUND_SOL" \
    --from "$DEMO_FUNDER_KEYPAIR" \
    --allow-unfunded-recipient \
    -u "$RPC_URL" \
    --commitment confirmed
}

# POST /onboard/provision → sign with seller → send (retry on stale blockhash / simulation).
provision_and_send() {
  local keypair="$1" asset="$2"
  local pk body http unsigned bh signed sig rpc_out rpc_err attempt max_attempts
  max_attempts="${PROVISION_SEND_ATTEMPTS:-6}"
  pk="$(solana address -k "$keypair")"
  body="$(jq -n --arg w "$pk" --arg a "$asset" '{wallet:$w, asset:$a}')"

  attempt=0
  while [[ "$attempt" -lt "$max_attempts" ]]; do
    attempt=$((attempt + 1))
    echo ">>> POST /onboard/provision wallet=$pk asset=$asset (send attempt $attempt/$max_attempts)"
    http="$(curl -sS -o "$WORKDIR/prov.json" -w "%{http_code}" \
      -X POST "$FACILITATOR_URL/api/v1/facilitator/onboard/provision" \
      -H "Content-Type: application/json" \
      -d "$body")"
    cat "$WORKDIR/prov.json" | jq .
    [[ "$http" == "200" ]] || {
      echo "❌ provision HTTP $http"
      return 1
    }
    if [[ "$(jq -r '.alreadyProvisioned' "$WORKDIR/prov.json")" == "true" ]]; then
      echo ">>> alreadyProvisioned=true — nothing to sign/send"
      return 0
    fi
    unsigned="$(jq -r '.transaction // empty' "$WORKDIR/prov.json")"
    bh="$(jq -r '.recentBlockhash // empty' "$WORKDIR/prov.json")"
    [[ -n "$unsigned" && -n "$bh" ]] || {
      echo "❌ missing transaction or recentBlockhash"
      return 1
    }
    echo ">>> Sign with seller keypair (e2e_sign_exact_tx)"
    cd "$PR402_ROOT"
    signed="$(printf '%s' "$unsigned" | cargo run -q --example e2e_sign_exact_tx -- "$keypair" "$bh")"
    echo ">>> sendTransaction (base64 bincode)"
    # Helius often simulates with BlockhashNotFound while the same tx lands with skipPreflight (matches facilitator blockhash).
    if [[ "${PROVISION_SKIP_PREFLIGHT:-1}" == "0" ]]; then
      rpc_out="$(curl -sS "$RPC_URL" -X POST -H "Content-Type: application/json" -d "$(jq -n \
        --arg b "$signed" \
        '{jsonrpc:"2.0", id:1, method:"sendTransaction", params:[$b, {encoding:"base64", skipPreflight:false, maxRetries:5}]}')")"
    else
      rpc_out="$(curl -sS "$RPC_URL" -X POST -H "Content-Type: application/json" -d "$(jq -n \
        --arg b "$signed" \
        '{jsonrpc:"2.0", id:1, method:"sendTransaction", params:[$b, {encoding:"base64", skipPreflight:true, maxRetries:5}]}')")"
    fi
    rpc_err="$(echo "$rpc_out" | jq -r '.error.message // empty')"
    sig="$(echo "$rpc_out" | jq -r '.result // empty')"
    if [[ -n "$sig" && "$sig" != "null" ]]; then
      echo ">>> Signature: $sig"
      solana confirm "$sig" -u "$RPC_URL" || true
      return 0
    fi
    echo "$rpc_out" | jq . >&2 || echo "$rpc_out" >&2
    if echo "$rpc_err" | grep -qiE 'Blockhash not found|BlockhashNotFound'; then
      echo "⚠️  Stale blockhash — refresh provision and retry"
      sleep 2
      continue
    fi
    echo "❌ sendTransaction failed"
    return 1
  done
  echo "❌ exhausted $max_attempts send attempts"
  return 1
}

expect_provision_http() {
  local keypair="$1" asset="$2" want="$3"
  local pk body http
  pk="$(solana address -k "$keypair")"
  body="$(jq -n --arg w "$pk" --arg a "$asset" '{wallet:$w, asset:$a}')"
  http="$(curl -sS -o "$WORKDIR/expect.json" -w "%{http_code}" \
    -X POST "$FACILITATOR_URL/api/v1/facilitator/onboard/provision" \
    -H "Content-Type: application/json" \
    -d "$body")"
  cat "$WORKDIR/expect.json" | jq . 2>/dev/null || cat "$WORKDIR/expect.json"
  [[ "$http" == "$want" ]] || {
    echo "❌ expected HTTP $want got $http"
    return 1
  }
}

# Register resource_providers row (USDC rail) via signed onboard — same as seller dashboard / agent flow.
onboard_registry_row() {
  local keypair="$1" asset="$2"
  local pk msg http ch_body expires now sig submit_body
  pk="$(solana address -k "$keypair")"
  echo ">>> GET /onboard/challenge?wallet=$pk"
  http="$(curl -sS -o "$WORKDIR/ch.json" -w "%{http_code}" \
    "$FACILITATOR_URL/api/v1/facilitator/onboard/challenge?wallet=$pk")"
  if [[ "$http" != "200" ]]; then
    cat "$WORKDIR/ch.json" | jq . 2>/dev/null || cat "$WORKDIR/ch.json"
    echo "⚠️  onboard challenge HTTP $http — cannot persist registry (need PR402_ONBOARD_HMAC_SECRET on facilitator)"
    return 1
  fi
  expires="$(jq -r '.expiresUnix' "$WORKDIR/ch.json")"
  now="$(date +%s)"
  if [[ "$now" -gt "$expires" ]]; then
    echo "❌ challenge already expired"
    return 1
  fi
  python3 - "$WORKDIR/ch.json" "$WORKDIR/onboard_msg.txt" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as f:
    msg = json.load(f)["message"]
with open(sys.argv[2], "wb") as out:
    out.write(msg.encode("utf-8"))
PY
  # Raw ed25519 over exact UTF-8 (not `solana sign-offchain-message`, which adds a domain prefix).
  cd "$PR402_ROOT"
  sig="$(cargo run -q --example e2e_sign_onboard_raw -- "$keypair" "$WORKDIR/onboard_msg.txt")"
  submit_body="$(jq -n \
    --arg w "$pk" \
    --rawfile m "$WORKDIR/onboard_msg.txt" \
    --arg s "$sig" \
    --arg a "$asset" \
    '{wallet:$w, message:$m, signature:$s, asset:$a}')"
  echo ">>> POST /onboard (registry persist)"
  http="$(curl -sS -o "$WORKDIR/ob.json" -w "%{http_code}" \
    -X POST "$FACILITATOR_URL/api/v1/facilitator/onboard" \
    -H "Content-Type: application/json" \
    -d "$submit_body")"
  cat "$WORKDIR/ob.json" | jq . 2>/dev/null || cat "$WORKDIR/ob.json"
  [[ "$http" == "200" ]] || {
    echo "❌ onboard submit HTTP $http"
    return 1
  }
  echo ">>> Registry row persisted for $pk ($asset)"
}

verify_three_accounts_from_provision_json() {
  local jsonf="$1"
  local v s a
  v="$(jq -r '.vaultPda // .vault_pda // empty' "$jsonf")"
  s="$(jq -r '.solStoragePda // .sol_storage_pda // empty' "$jsonf")"
  a="$(jq -r '.vaultTokenAta // .vault_token_ata // empty' "$jsonf")"
  [[ "$a" == "null" ]] && a=""
  [[ -n "$v" && -n "$s" ]] || {
    echo "❌ provision JSON missing vault / sol storage (file $jsonf)"
    return 1
  }
  assert_account "SplitVault" "$v"
  assert_account "SOL storage PDA" "$s"
  if [[ -n "$a" ]]; then
    assert_account "vault SPL ATA" "$a"
  else
    echo "❌ expected vault_token_ata in response for SPL provision"
    return 1
  fi
}

echo ""
echo "################################################################################"
echo "# E2E START | seller provision | POST /onboard/provision (devnet)"
echo "# Facilitator: $FACILITATOR_URL"
echo "# RPC:         $RPC_URL"
echo "################################################################################"

echo ">>> Preflight: facilitator health"
curl -sS "$FACILITATOR_URL/api/v1/facilitator/health" | jq '{status, environment, solanaNetwork, database}' || true

echo ""
echo ">>> Preflight: custom mint on cluster (scenario C)"
solana account "$E2E_CUSTOM_DEVNET_MINT" -u "$RPC_URL" | head -6 || {
  echo "❌ Custom mint not found on $RPC_URL — set E2E_CUSTOM_DEVNET_MINT"
  exit 1
}

# --- Wallet A: USDC ---
A_JSON="$WORKDIR/A.json"
A_PK="$(new_seller_keypair "$A_JSON")"
fund_seller "$A_PK"
provision_and_send "$A_JSON" "USDC"
# Re-fetch provision state (idempotent) to read canonical PDAs in JSON
curl -sS -o "$WORKDIR/A_prov.json" -X POST "$FACILITATOR_URL/api/v1/facilitator/onboard/provision" \
  -H "Content-Type: application/json" \
  -d "$(jq -n --arg w "$A_PK" --arg a "USDC" '{wallet:$w, asset:$a}')"
jq . "$WORKDIR/A_prov.json"
echo ">>> Scenario A: assert three on-chain accounts (USDC rail)"
verify_three_accounts_from_provision_json "$WORKDIR/A_prov.json"

# --- Wallet A2: policy (optional) ---
if [[ "$SKIP_SELLER_PROVISION_DB_POLICY" != "1" ]]; then
  echo ""
  echo ">>> Scenario A2: persist USDC rail in DB, then expect 400 for WSOL provision"
  if onboard_registry_row "$A_JSON" "USDC"; then
    if expect_provision_http "$A_JSON" "WSOL" "400"; then
      err_txt="$(jq -r '.error // empty' "$WORKDIR/expect.json")"
      if echo "$err_txt" | grep -qiE 'different payment asset|already registered|resource_providers|one asset|merchant wallet'; then
        echo "✓ Scenario A2: second-rail provision rejected as expected"
      else
        echo "⚠️  HTTP 400 but message unexpected: $err_txt"
      fi
    else
      echo "❌ Scenario A2: expected HTTP 400 for second rail"
      exit 1
    fi
  else
    echo "⚠️  Skip A2: onboard/registry unavailable (set SKIP_SELLER_PROVISION_DB_POLICY=1 to silence)"
  fi
else
  echo ">>> SKIP_SELLER_PROVISION_DB_POLICY=1 — skip A2"
fi

# --- Wallet B: WSOL ---
B_JSON="$WORKDIR/B.json"
B_PK="$(new_seller_keypair "$B_JSON")"
fund_seller "$B_PK"
echo ""
echo ">>> Scenario B: initial provision with WSOL (wrapped-SOL mint preset)"
provision_and_send "$B_JSON" "WSOL"
curl -sS -o "$WORKDIR/B_prov.json" -X POST "$FACILITATOR_URL/api/v1/facilitator/onboard/provision" \
  -H "Content-Type: application/json" \
  -d "$(jq -n --arg w "$B_PK" --arg a "WSOL" '{wallet:$w, asset:$a}')"
jq . "$WORKDIR/B_prov.json"
echo ">>> Observation: same SplitVault + sol storage as USDC path; vault ATA is for So111… (WSOL)."
verify_three_accounts_from_provision_json "$WORKDIR/B_prov.json"

# --- Wallet C: arbitrary mint ---
C_JSON="$WORKDIR/C.json"
C_PK="$(new_seller_keypair "$C_JSON")"
fund_seller "$C_PK"
echo ""
echo ">>> Scenario C: initial provision mint $E2E_CUSTOM_DEVNET_MINT"
http_c="$(curl -sS -o "$WORKDIR/C_prov.json" -w "%{http_code}" \
  -X POST "$FACILITATOR_URL/api/v1/facilitator/onboard/provision" \
  -H "Content-Type: application/json" \
  -d "$(jq -n --arg w "$C_PK" --arg a "$E2E_CUSTOM_DEVNET_MINT" '{wallet:$w, asset:$a}')")"
jq . "$WORKDIR/C_prov.json" 2>/dev/null || cat "$WORKDIR/C_prov.json"
if [[ "$http_c" != "200" ]]; then
  _cerr="$(jq -r '.error // empty' "$WORKDIR/C_prov.json")"
  if echo "$_cerr" | grep -qiE 'not supported for payment|Approved assets'; then
    echo "⚠️  Scenario C skipped: mint not in facilitator allowlist (expected on many deployments)."
  else
    echo "❌ Scenario C provision HTTP $http_c"
    exit 1
  fi
else
  if [[ "$(jq -r '.alreadyProvisioned' "$WORKDIR/C_prov.json")" != "true" ]]; then
    provision_and_send "$C_JSON" "$E2E_CUSTOM_DEVNET_MINT" || exit 1
  fi
  curl -sS -o "$WORKDIR/C_prov.json" -X POST "$FACILITATOR_URL/api/v1/facilitator/onboard/provision" \
    -H "Content-Type: application/json" \
    -d "$(jq -n --arg w "$C_PK" --arg a "$E2E_CUSTOM_DEVNET_MINT" '{wallet:$w, asset:$a}')"
  jq . "$WORKDIR/C_prov.json"
  verify_three_accounts_from_provision_json "$WORKDIR/C_prov.json"
fi

echo ""
echo "✅ Seller provision E2E completed (A + B on-chain; C if allowlist permits)."
echo ""
echo "################################################################################"
echo "# E2E END   | seller provision"
echo "################################################################################"
