#!/usr/bin/env bash
#
# Full devnet x402 E2E (both rails, verify + settle):
#   Scenario A: UniversalSettle `exact` — default 0.05 USDC (`E2E_SCENARIO_A_AMOUNT_RAW`)
#   Scenario B1: SLA-Escrow CLI (buyer-paid fees) — `01_...sh`
#   Scenario B2: SLA-Escrow HTTP build (facilitator-paid fees) — `03_...sh`
#
# Usage:
#   export RPC_URL="https://devnet.helius-rpc.com/?api-key=..."
#   export FACILITATOR_URL="https://preview.pr402.signer-payer.me"
#   ./run_all_devnet.sh
#
# Flags:
#   SKIP_EXACT=1       — scenario A off
#   SKIP_SLA=1         — both SLA scenarios (B1 + B2) off
#   SKIP_SLA_HTTP=1    — skip B2 (facilitator fees / HTTP build only)
#   SKIP_SLA_CLI=1     — skip B1 (CLI fund-payment / buyer-paid only)
#   RUN_SLA_LIFECYCLE=1 — after B1, run post-fund lifecycle (sets E2E_SLA_FULL_LIFECYCLE; needs oracle key + DB migration 003)
#   RUN_FULL_SLA_LIFECYCLE=1 — only `05_sla_escrow_full_cycle_devnet.sh` (fund…release + DB; skips B2/B1 orchestration here)
#
set -euo pipefail
HERE="$(dirname "$0")"

echo "================================================================"
echo " pr402 devnet E2E — B2 (SLA HTTP) + B1 (SLA CLI) + A (exact); amounts in common.sh"
echo "================================================================"
echo ""

if [[ "${SKIP_SLA:-}" != "1" ]]; then
  if [[ "${RUN_FULL_SLA_LIFECYCLE:-}" == "1" ]]; then
    echo ">>> RUN_FULL_SLA_LIFECYCLE=1 — single full SLA cycle (05: B1 + verify + settle + delivery + oracle + release + DB)"
    "$HERE/05_sla_escrow_full_cycle_devnet.sh"
  else
    if [[ "${RUN_SLA_LIFECYCLE:-}" == "1" ]]; then
      export E2E_SLA_FULL_LIFECYCLE=1
    fi
    if [[ "${SKIP_SLA_HTTP:-}" != "1" ]]; then
      echo ">>> Scenario B2: SLA-Escrow facilitator fees (build-sla-escrow-payment-tx)"
      "$HERE/03_sla_escrow_http_facilitator_fees.sh"
    else
      echo ">>> SKIP_SLA_HTTP=1 — skip Scenario B2"
    fi
    echo ""
    if [[ "${SKIP_SLA_CLI:-}" != "1" ]]; then
      echo ">>> Scenario B1: SLA-Escrow buyer-paid fees (CLI fund-payment)"
      "$HERE/01_sla_escrow_facilitator_verify.sh"
    else
      echo ">>> SKIP_SLA_CLI=1 — skip Scenario B1"
    fi
  fi
else
  echo ">>> SKIP_SLA=1 — skip Scenario B (B1 + B2)"
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
