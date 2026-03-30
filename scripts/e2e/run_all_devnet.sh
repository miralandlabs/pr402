#!/usr/bin/env bash
#
# Run full devnet E2E against pr402 HTTP: sla-escrow verify + exact verify.
#
# Usage:
#   export RPC_URL="https://devnet.helius-rpc.com/?api-key=..."
#   export FACILITATOR_URL="https://preview.pr402.signer-payer.me"
#   ./run_all_devnet.sh
#
# Flags:
#   SKIP_EXACT=1   — only run SLA-Escrow path
#   SKIP_SLA=1     — only run exact path
#
set -euo pipefail
HERE="$(dirname "$0")"

if [[ "${SKIP_SLA:-}" != "1" ]]; then
  "$HERE/01_sla_escrow_facilitator_verify.sh"
else
  echo ">>> SKIP_SLA=1 — skip 01_sla_escrow_facilitator_verify.sh"
fi

if [[ "${SKIP_EXACT:-}" != "1" ]]; then
  "$HERE/02_exact_facilitator_verify.sh"
else
  echo ">>> SKIP_EXACT=1 — skip 02_exact_facilitator_verify.sh"
fi

echo ""
echo "🎉 run_all_devnet: all enabled phases completed."
