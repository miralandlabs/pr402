# shellcheck shell=bash
# Shared layout + defaults for pr402 devnet E2E scripts.
# Source from siblings:  source "$(dirname "$0")/common.sh"

E2E_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PR402_ROOT="$(cd "$E2E_ROOT/../.." && pwd)"
WORKSPACE_ROOT="$(cd "$PR402_ROOT/.." && pwd)"

export FACILITATOR_URL="${FACILITATOR_URL:-https://preview.agent.pay402.me}"
export RPC_URL="${RPC_URL:-${SOLANA_RPC_URL:-https://api.devnet.solana.com}}"
# Circle USDC on devnet (same family as scripts elsewhere in the monorepo)
export E2E_USDC_MINT="${E2E_USDC_MINT:-4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU}"
export DEVNET_CHAIN_ID="${DEVNET_CHAIN_ID:-solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1}"

export SLA_ESCROW_SCRIPTS="$WORKSPACE_ROOT/sla-escrow/scripts"
export UNIVERSALSETTLE_CLI="${UNIVERSALSETTLE_CLI:-$WORKSPACE_ROOT/universalsettle/target/release/universalsettle}"
export SLA_ESCROW_CLI="${SLA_ESCROW_CLI:-$WORKSPACE_ROOT/sla-escrow/target/release/sla-escrow}"

export E2E_BUYER_KEYPAIR="${E2E_BUYER_KEYPAIR:-$HOME/.config/solana/id.json}"
export E2E_SELLER_KEYPAIR="${E2E_SELLER_KEYPAIR:-$HOME/.config/solana/test-id.json}"
export E2E_ADMIN_KEYPAIR="${E2E_ADMIN_KEYPAIR:-$HOME/.config/solana/id.json}"

# Production-style routing (reference): amount < threshold → UniversalSettle (`exact`);
# amount >= threshold → SLA-Escrow. Devnet defaults below use **small** amounts for cheap test wallets.
export USDC_POLICY_THRESHOLD_WHOLE="${USDC_POLICY_THRESHOLD_WHOLE:-10}"
# Scenario A: `exact` path — raw USDC (6 decimals). Default 0.05 USDC (= 50_000 raw).
export E2E_SCENARIO_A_AMOUNT_RAW="${E2E_SCENARIO_A_AMOUNT_RAW:-50000}"
# Scenario B: `sla-escrow` fund-payment — human USDC string. Default 1 USDC.
export E2E_SCENARIO_B_AMOUNT_HUMAN="${E2E_SCENARIO_B_AMOUNT_HUMAN:-1}"

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "❌ missing command: $1"
    exit 1
  }
}

seller_pubkey() {
  solana address -k "$E2E_SELLER_KEYPAIR"
}

buyer_pubkey() {
  solana address -k "$E2E_BUYER_KEYPAIR"
}

# Parse DATABASE_URL from pr402/.env (spaces around `=` tolerated).
load_database_url_from_env_file() {
  local envf="$PR402_ROOT/.env"
  [[ -f "$envf" ]] || return 0
  DATABASE_URL="$(
    python3 - "$envf" <<'PY'
import re, sys
from pathlib import Path
path = Path(sys.argv[1])
for line in path.read_text(encoding="utf-8").splitlines():
    line = line.strip()
    if not line or line.startswith("#"):
        continue
    m = re.match(r"^DATABASE_URL\s*=\s*(.+)\s*$", line)
    if not m:
        continue
    val = m.group(1).strip().strip('"').strip("'")
    print(val, end="")
    break
PY
  )"
  export DATABASE_URL
}

psql_audit_for_correlation() {
  local cid="$1"
  [[ -n "${DATABASE_URL:-}" ]] || {
    echo ">>> DATABASE_URL unset; skip DB audit"
    return 0
  }
  echo ">>> DB: payment_attempts + escrow_details for $cid"
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c "
SELECT pa.correlation_id, pa.scheme, pa.verify_ok, pa.settle_ok,
       pa.settlement_signature,
       ed.escrow_pda, ed.oracle_authority, ed.fund_signature, ed.sla_hash
FROM payment_attempts pa
LEFT JOIN escrow_details ed ON ed.payment_attempt_id = pa.id
WHERE pa.correlation_id = '${cid//\'/\'\'}';
"
}

# Extract Solana tx signature line from sla-escrow CLI stdout (fund, submit-delivery, confirm-oracle, …).
parse_sla_escrow_tx_sig() {
  echo "$1" | grep "📝 Transaction signature:" | sed 's/.*📝 Transaction signature: //' | head -1 | tr -d '\r' | tr -d ' '
}

# Lifecycle columns + last event (needs migration 003_escrow_lifecycle_events.sql on the DB).
psql_audit_escrow_lifecycle() {
  local cid="$1"
  [[ -n "${DATABASE_URL:-}" ]] || {
    echo ">>> DATABASE_URL unset; skip lifecycle DB audit"
    return 0
  }
  echo ">>> DB: escrow_details lifecycle + events for $cid"
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c "
SELECT ed.delivery_hash, ed.delivery_signature, ed.resolution_signature, ed.resolution_state,
       ed.completed_at, ed.refunded_at
FROM payment_attempts pa
JOIN escrow_details ed ON ed.payment_attempt_id = pa.id
WHERE pa.correlation_id = '${cid//\'/\'\'}';
SELECT e.step, e.tx_signature, e.payload, e.created_at
FROM payment_attempts pa
JOIN escrow_lifecycle_events e ON e.payment_attempt_id = pa.id
WHERE pa.correlation_id = '${cid//\'/\'\'}'
ORDER BY e.id ASC;
"
}

# x402 v2: SettleRequest uses the same JSON shape as VerifyRequest (see pr402 `proto::SettleRequest`).
# Usage: facilitator_settle "$VERIFY_BODY_JSON" "$CORRELATION_ID" /tmp/out.json → prints HTTP code, body in file
facilitator_settle() {
  local body="$1"
  local cid="$2"
  local out="${3:-/tmp/e2e_settle_out.json}"
  curl -sS -o "$out" -w "%{http_code}" \
    -X POST "${FACILITATOR_URL}/api/v1/facilitator/settle" \
    -H "Content-Type: application/json" \
    -H ": $cid" \
    -d "$body"
}
