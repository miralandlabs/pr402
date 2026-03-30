#!/usr/bin/env bash
#
# verify-audit-persistence.sh
# Verifies that pr402 facilitator correctly captures enriched metadata to the DB.
#

set -euo pipefail

# Configuration
RPC="https://devnet.helius-rpc.com/?api-key=5207c547-a878-46ef-892d-cae1446de8bf"
MINT="4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
FACILITATOR_URL="http://127.0.0.1:3000"
DB_URL=$(grep "DATABASE_URL" .env | head -1 | cut -d'"' -f2)

echo "=============================================="
echo " 🛡️ pr402 Audit Persistence Verification"
echo "=============================================="
echo " DB: $DB_URL"
echo ""

# 1. Start Facilitator in background
echo "Step 1: Starting local Facilitator..."
cargo build --release --bin facilitator
export DATABASE_URL="$DB_URL"
export SOLANA_RPC_URL="$RPC"
# Use temporary port to avoid collisions
export PORT=3000
./target/release/facilitator > facilitator.log 2>&1 &
FAC_PID=$!

cleanup() {
  echo ">>> Cleaning up facilitator (PID: $FAC_PID)"
  kill $FAC_PID || true
}
trap cleanup EXIT

# Wait for facilitator to start
echo "Waiting for facilitator to spin up..."
sleep 5

# 2. Perform /verify for sla-escrow
echo "Step 2: Sending SLA-Escrow Verify Request..."
CORRELATION_ID="audit-test-$(date +%s)"

curl -s -X POST "$FACILITATOR_URL/api/v1/facilitator/verify" \
  -H "Content-Type: application/json" \
  -H "X-Correlation-Id: $CORRELATION_ID" \
  -d '{
    "correlationId": "'$CORRELATION_ID'",
    "intent": {
      "kinds": [{
        "scheme": "sla-escrow",
        "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
        "extra": {
          "bankAddress": "8aNEFX3A6p4e1PqKDkcL6xVa1X5zdHj2BgXh8QitqdX6",
          "configAddress": "B3SchPvtYtFwseZGkgxdhyt3z1dLVs6AbhEv1gGJt8Rj",
          "escrowProgramId": "s1jWKnB1QwKKKZUDq3bZCmqvwEf8UQpQCbkEtQzHknS",
          "feeBps": 100,
          "feePayer": "FaciLFwHjbW9V1PtF3vAweL1K1hgin9mvXNXatEQKJdu",
          "oracleAuthorities": ["FaciLFwHjbW9V1PtF3vAweL1K1hgin9mvXNXatEQKJdu"],
          "ttlSeconds": 3600
        }
      }],
      "signers": {
        "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1": ["FaciLFwHjbW9V1PtF3vAweL1K1hgin9mvXNXatEQKJdu"]
      }
    },
    "paymentRequirements": {
      "payTo": "BLpLZ8QFPv9hKsvpKE6SeZuoyrAHGySE8gVGZs7TvbtZ",
      "amount": "15.0",
      "asset": "'$MINT'",
      "memo": "Institutional Model Training SLA"
    }
  }' | jq .

echo ""
echo "✅ Verify Request Sent."
echo "Step 3: Checking Database for record..."
sleep 2

# Use psql explicitly to check the row
# Note: we use the DB_URL from .env
PGPASSWORD=$(echo $DB_URL | sed 's/.*:\(.*\)@.*/\1/' | sed 's/%21/!/g' | sed 's/%24/$/g')
DB_HOST=$(echo $DB_URL | sed 's/.*@\(.*\):.*/\1/')
DB_USER=$(echo $DB_URL | sed 's/postgresql:\/\/\([^:]*\).*/\1/')
DB_PORT=$(echo $DB_URL | sed 's/.*:\([0-9]*\)\/.*/\1/')

psql -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" -d postgres -c "SELECT correlation_id, scheme, amount, asset, verify_ok FROM payment_attempts WHERE correlation_id = '$CORRELATION_ID';"

echo ""
echo "Step 4: Checking Escrow Details extension..."
psql -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" -d postgres -c "SELECT * FROM escrow_details WHERE correlation_id = '$CORRELATION_ID';"

echo "=============================================="
echo " 🎉 Audit Verification Finished."
echo "=============================================="
