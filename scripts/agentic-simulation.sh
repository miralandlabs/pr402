#!/usr/bin/env bash
#
# Agentic End-to-End Simulation: Dynamic Settlement based on Amount
#
# Scenario A: Amount < $10 USDC  -> UniversalSettle (Exact)
# Scenario B: Amount >= $10 USDC -> SLAEscrow (Escrow)
#
# Roles:
#  - Buyer Agent: Receives 402, requests payment from their wallet.
#  - Seller Agent (RP): Onboards vaults, choose scheme for the task.
#  - Facilitator: Public metadata discovery + Transaction verification/settlement.

set -euo pipefail

# Configuration
MINT="4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU" # USDC on Devnet
RPC="https://devnet.helius-rpc.com/?api-key=5207c547-a878-46ef-892d-cae1446de8bf"
CHAIN_ID="solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1" # Devnet

# Standard Keypairs
ADMIN_KP="$HOME/.config/solana/id.json"
FACILITATOR_KP="$HOME/.config/solana/id.json"
ORACLE_KP="$HOME/.config/solana/id.json"

# Program IDs
UNIVERSALSETTLE_ID="u2pHjM1sFzXFYK9Sh6hdjx1uVDW7pVUfUeCfpudrYu7"
SLA_ESCROW_ID="s1jWKnB1QwKKKZUDq3bZCmqvwEf8UQpQCbkEtQzHknS"

# CLIs
UNIVERSALSETTLE_CLI="../universalsettle/target/release/universalsettle"
SLA_ESCROW_CLI="../sla-escrow/target/release/sla-escrow"
PR402_CLI="./target/release/pr402"

echo "=============================================="
echo " 🌌 Agentic Payment Simulation (x402 V2)"
echo "=============================================="
echo " RPC: $RPC"
echo " Mint: $MINT"
echo ""

# 1. Preparation
echo "Step 1: Building CLIs..."
cargo build --release --package pr402
(cd ../universalsettle && cargo build --release)
(cd ../sla-escrow && cargo build --release --package sla-escrow-cli --features admin)
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

# 3. Decision Logic Demo
simulate_interaction() {
  local amount=$1
  local task=$2
  echo "--- Testing Scenario: $task ($amount USDC) ---"

  if (( $(echo "$amount < 10.0" | bc -l) )); then
    echo "💡 Decision: Small amount detected. Using UniversalSettle (Exact)."
    SCHEME="exact"
    ./../universalsettle/scripts/e2e-spl-test.sh --mint "$MINT" --amount "$amount" --rpc "$RPC"
  else
    echo "🛡️ Decision: Large amount detected. Using SLAEscrow (Escrow)."
    SCHEME="sla-escrow"
    ./../sla-escrow/scripts/test-usdc-e2e.sh --rpc "$RPC"
    # Note: test-usdc-e2e.sh uses its own internal UIDs, but we can customize it easily.
  fi
  echo ""
}

# Run Scenario A: Small Amount
simulate_interaction 1.0 "Haiku Generation Task"

# Run Scenario B: Large Amount
simulate_interaction 15.0 "Complex Model Training Task"

echo "=============================================="
echo " 🎉 Agentic Simulation Successful!"
echo "=============================================="
