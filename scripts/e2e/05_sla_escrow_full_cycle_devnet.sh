#!/usr/bin/env bash
#
# Single end-to-end devnet run: SLA fund (B1) → /verify → /settle → submit-delivery →
# confirm-oracle → release-payment (or refund if E2E_SLA_LIFECYCLE_REFUND=1), plus DB lifecycle rows.
#
# Oracle must be a keypair you own. Defaults E2E_ORACLE_KEYPAIR to E2E_BUYER_KEYPAIR so one wallet
# can act as buyer + oracle on devnet (not a production pattern).
#
# Requires: same as 01 + 04 — sla-escrow CLI (admin for open-escrow), DATABASE_URL, migration 003,
# funded buyer USDC + SOL, RPC_URL, FACILITATOR_URL.
#
# Env:
#   E2E_ORACLE_KEYPAIR — optional; default E2E_BUYER_KEYPAIR
#   E2E_SLA_LIFECYCLE_REFUND=1 — run refund path after oracle reject (see 04)
#   (all 01 / 04 / common.sh vars apply)
#
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/common.sh"

require_cmd solana
require_cmd curl
require_cmd jq
require_cmd psql
require_cmd python3

load_database_url_from_env_file
[[ -n "${DATABASE_URL:-}" ]] || {
  echo "❌ DATABASE_URL required (export or pr402/.env) for escrow_details + escrow_lifecycle_events"
  exit 1
}

export E2E_ORACLE_KEYPAIR="${E2E_ORACLE_KEYPAIR:-$E2E_BUYER_KEYPAIR}"
[[ -f "$E2E_ORACLE_KEYPAIR" ]] || {
  echo "❌ E2E_ORACLE_KEYPAIR not found (and E2E_BUYER_KEYPAIR missing for default)"
  exit 1
}
export E2E_ORACLE_AUTHORITY="$(solana address -k "$E2E_ORACLE_KEYPAIR")"

echo "================================================================"
echo " E2E: FULL SLA-Escrow devnet (fund → settle → delivery → oracle → release/refund + DB)"
echo "  Oracle (fund + confirm): $E2E_ORACLE_AUTHORITY"
echo "  Facilitator: $FACILITATOR_URL"
echo "  Refund path: ${E2E_SLA_LIFECYCLE_REFUND:-0}"
echo "================================================================"
echo ""

export E2E_SLA_FULL_LIFECYCLE=1
"$HERE/01_sla_escrow_facilitator_verify.sh"

echo ""
echo "🎉 05: Full SLA-Escrow cycle completed (including on-chain release or refund when enabled)."
