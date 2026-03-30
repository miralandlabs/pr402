#!/usr/bin/env bash
#
# Full devnet x402 E2E (both rails, verify + settle):
#   Scenario A: UniversalSettle `exact` — default 0.05 USDC (`E2E_SCENARIO_A_AMOUNT_RAW`)
#   Scenario B: SLA-Escrow — default 1 USDC (`E2E_SCENARIO_B_AMOUNT_HUMAN`)
#
# Usage:
#   export RPC_URL="https://devnet.helius-rpc.com/?api-key=..."
#   export FACILITATOR_URL="https://preview.pr402.signer-payer.me"
#   ./run_all_devnet.sh
#
# Flags:
#   SKIP_EXACT=1   — scenario A only off
#   SKIP_SLA=1     — scenario B only off
#
set -euo pipefail
HERE="$(dirname "$0")"

echo "================================================================"
echo " pr402 devnet E2E — Scenario B (sla-escrow) then A (exact); amounts in common.sh"
echo "================================================================"
echo ""

if [[ "${SKIP_SLA:-}" != "1" ]]; then
  echo ">>> Scenario B: SLA-Escrow (~1 USDC default)"
  "$HERE/01_sla_escrow_facilitator_verify.sh"
else
  echo ">>> SKIP_SLA=1 — skip Scenario B"
fi

echo ""

if [[ "${SKIP_EXACT:-}" != "1" ]]; then
  echo ">>> Scenario A: UniversalSettle / exact (~0.05 USDC default)"
  "$HERE/02_exact_facilitator_verify.sh"
else
  echo ">>> SKIP_EXACT=1 — skip Scenario A"
fi

echo ""
echo "🎉 run_all_devnet: enabled scenarios completed."
