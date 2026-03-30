#!/usr/bin/env bash
#
# Agentic End-to-End Simulation: Dynamic Settlement based on Amount
#
# Scenario A: Amount < $10 USDC  -> UniversalSettle (Exact) — on-chain SPL vault E2E
# Scenario B: Amount >= $10 USDC -> SLAEscrow (Escrow) — on-chain escrow lifecycle E2E
#
# Roles:
#  - Buyer Agent: Receives 402, requests payment from their wallet.
#  - Seller Agent (RP): Onboards vaults, choose scheme for the task.
#  - Facilitator: Public metadata discovery + Transaction verification/settlement.
#
# Run from anywhere: paths resolve via this script's location (repo layout: x402/pr402, x402/universalsettle, x402/sla-escrow).
# Requires: bc, solana CLI, spl-token (exact path), release CLIs built.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PR402_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WORKSPACE_ROOT="$(cd "$PR402_ROOT/.." && pwd)"

# Configuration
MINT="4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU" # USDC (Official Circle) on Devnet
RPC="https://devnet.helius-rpc.com/?api-key=5207c547-a878-46ef-892d-cae1446de8bf"
CHAIN_ID="solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1" # Devnet

# Threshold (USDC): < this -> UniversalSettle; >= this -> SLA-Escrow
USDC_POLICY_THRESHOLD="${USDC_POLICY_THRESHOLD:-10.0}"

# Standard Keypairs
ADMIN_KP="$HOME/.config/solana/id.json"

# CLIs (sibling repos under WORKSPACE_ROOT)
UNIVERSALSETTLE_CLI="$WORKSPACE_ROOT/universalsettle/target/release/universalsettle"
SLA_ESCROW_CLI="$WORKSPACE_ROOT/sla-escrow/target/release/sla-escrow"

echo "=============================================="
echo " 🌌 Agentic Payment Simulation (x402 V2)"
echo "=============================================="
echo " Workspace: $WORKSPACE_ROOT"
echo " RPC: $RPC"
echo " Mint: $MINT"
echo " Policy: < ${USDC_POLICY_THRESHOLD} USDC -> UniversalSettle | >= -> SLA-Escrow"
echo ""

# 1. Preparation
echo "Step 1: Building CLIs..."
cargo build --release --manifest-path "$PR402_ROOT/Cargo.toml" --package pr402
(cargo build --release --manifest-path "$WORKSPACE_ROOT/universalsettle/Cargo.toml")
(cargo build --release --manifest-path "$WORKSPACE_ROOT/sla-escrow/Cargo.toml" --package sla-escrow-cli --features admin)
echo "✅ Build complete."
echo ""

# 2. Seller Onboarding
echo "Step 2: Onboarding Seller Agent (Dynamic Vaults)..."
SELLER_KP=$(mktemp /tmp/seller-XXXXXX.json)
solana-keygen new --no-bip39-passphrase -s -o "$SELLER_KP" --force
SELLER_PUB=$(solana address -k "$SELLER_KP")
echo "   Seller: $SELLER_PUB"

# a) UniversalSettle Vault (for small payments)
$UNIVERSALSETTLE_CLI create-vault \
  --seller "$SELLER_PUB" \
  --fee-destination "$SELLER_PUB" \
  --fee-bps 100 \
  --rpc "$RPC" --keypair "$ADMIN_KP" --yes

# b) SLAEscrow Bank (already exists on devnet usually, but we ensure bank 0)
# Usually Bank is global, but we ensure seller has knowledge of it.
echo "   Ensuring SLAEscrow Bank 0 exists..."
$SLA_ESCROW_CLI initialize --fee-bps 100 --rpc "$RPC" --keypair "$ADMIN_KP" --yes || true
echo "✅ Onboarding Complete."
echo ""

command -v bc >/dev/null || {
  echo "❌ Required: bc (for floating compare). e.g. brew install bc"
  exit 1
}

# 3. Decision Logic Demo
simulate_interaction() {
  local amount=$1
  local task=$2
  echo "--- Testing Scenario: $task ($amount USDC) ---"

  if (( $(echo "$amount < $USDC_POLICY_THRESHOLD" | bc -l) )); then
    echo "💡 Decision: Amount < ${USDC_POLICY_THRESHOLD} USDC → UniversalSettle (exact / vault SPL path)."
    SCHEME="exact"
    "$WORKSPACE_ROOT/universalsettle/scripts/e2e-spl-test.sh" --mint "$MINT" --amount "$amount" --rpc "$RPC"
  else
    echo "🛡️ Decision: Amount >= ${USDC_POLICY_THRESHOLD} USDC → SLAEscrow (sla-escrow path)."
    SCHEME="sla-escrow"
    # test-usdc-e2e.sh runs fixed 0.1 USDC steps; the *policy* here is "large payment → escrow rail", not dollar-matched step sizes.
    "$WORKSPACE_ROOT/sla-escrow/scripts/test-usdc-e2e.sh" --rpc "$RPC"
  fi
  echo ""
}

# Run Scenario A: Small Amount (< $10 USDC → UniversalSettle)
simulate_interaction 1.0 "Haiku Generation Task"

# Run Scenario B: Large Amount (>= $10 USDC → SLA-Escrow)
simulate_interaction 15.0 "Complex Model Training Task"

echo "=============================================="
echo " 🎉 Agentic Simulation Successful!"
echo "=============================================="
