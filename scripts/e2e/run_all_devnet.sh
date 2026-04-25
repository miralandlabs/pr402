#!/usr/bin/env bash
#
# Full devnet x402 E2E (verify + settle):
#   Facilitator pays Solana network fees (two rails):
#     • Scenario A: UniversalSettle `exact` — `02_...sh` (build-exact-payment-tx, feePayer = facilitator)
#     • Scenario B2: SLA-Escrow HTTP — `03_...sh` (build-sla-escrow-payment-tx, default facilitator fee payer)
#   Buyer pays network fees:
#     • Scenario B1: SLA-Escrow CLI fund-payment — `01_...sh`
#
# Usage:
#   export RPC_URL="https://devnet.helius-rpc.com/?api-key=..."
#   export FACILITATOR_URL="https://preview.agent.pay402.me"
#   ./run_all_devnet.sh
#
# Flags:
#   SKIP_EXACT=1       — scenario A off
#   SKIP_SLA=1         — both SLA scenarios (B1 + B2) off
#   SKIP_SLA_HTTP=1    — skip B2 (facilitator fees / HTTP build only)
#   SKIP_SLA_CLI=1     — skip B1 (CLI fund-payment / buyer-paid only)
#   RUN_SLA_LIFECYCLE=1 — after B1, run post-fund lifecycle (sets E2E_SLA_FULL_LIFECYCLE; needs oracle key + DB migration 003)
#   RUN_FULL_SLA_LIFECYCLE=1 — B2 (unless SKIP_SLA_HTTP) → 05 (B1 fund + verify/settle + 04 lifecycle + DB) → A
#
set -euo pipefail
HERE="$(dirname "$0")"

echo "================================================================"
if [[ "${RUN_FULL_SLA_LIFECYCLE:-}" == "1" ]]; then
  echo " pr402 devnet E2E — RUN_FULL_SLA_LIFECYCLE=1"
  echo "   Order: B2 (facilitator fee SLA-HTTP, unless SKIP_SLA_HTTP) → 05 (B1 + full on-chain lifecycle) → A (exact)"
else
  echo " pr402 devnet E2E — order: B2 → B1 → A (amounts in common.sh)"
fi
echo "================================================================"
echo " Facilitator pays Solana tx fees — TWO rails:"
echo "   B2 — SLA-Escrow — 03_sla_escrow_http_facilitator_fees.sh  (search: E2E START | B2)"
echo "   A  — exact      — 02_exact_facilitator_verify.sh           (search: E2E START | A)"
echo " Buyer pays Solana tx fees — SLA CLI:"
echo "   B1 — 01_sla_escrow_facilitator_verify.sh                  (search: E2E START | B1)"
echo "================================================================"
echo ""

if [[ "${SKIP_SLA:-}" != "1" ]]; then
  if [[ "${RUN_FULL_SLA_LIFECYCLE:-}" == "1" ]]; then
    if [[ "${SKIP_SLA_HTTP:-}" != "1" ]]; then
      echo ">>> RUN_FULL_SLA_LIFECYCLE=1 — invoking B2 first (SLA-Escrow, facilitator fee payer / HTTP)"
      "$HERE/03_sla_escrow_http_facilitator_fees.sh"
      echo ""
    else
      echo ">>> RUN_FULL_SLA_LIFECYCLE=1 — SKIP_SLA_HTTP=1 (B2 skipped); then 05"
    fi
    echo ">>> RUN_FULL_SLA_LIFECYCLE=1 — invoking 05 (B1-based fund + verify/settle + post-fund lifecycle + DB)"
    "$HERE/05_sla_escrow_full_cycle_devnet.sh"
  else
    if [[ "${RUN_SLA_LIFECYCLE:-}" == "1" ]]; then
      export E2E_SLA_FULL_LIFECYCLE=1
    fi
    if [[ "${SKIP_SLA_HTTP:-}" != "1" ]]; then
      echo ">>> Invoking B2 (facilitator fee payer / HTTP) — see E2E START | B2 inside script output"
      "$HERE/03_sla_escrow_http_facilitator_fees.sh"
    else
      echo ">>> SKIP_SLA_HTTP=1 — skip Scenario B2"
    fi
    echo ""
    if [[ "${SKIP_SLA_CLI:-}" != "1" ]]; then
      echo ">>> Invoking B1 (buyer-paid fees / CLI) — see E2E START | B1 inside script output"
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
  echo ">>> Invoking A (exact / UniversalSettle, facilitator fee payer) — see E2E START | A"
  "$HERE/02_exact_facilitator_verify.sh"
else
  echo ">>> SKIP_EXACT=1 — skip Scenario A"
fi

echo ""
echo "🎉 run_all_devnet: enabled scenarios completed."
