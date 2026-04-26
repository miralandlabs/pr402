# Buyer Quick Start — 6 Steps to Pay via x402

> **You have a Solana wallet and received HTTP 402 from a seller. Here's what to do.**

> **Launch phase:** **Experimental** — **use at your own risk**.

Replace **`$BASE`** with the facilitator URL the seller documents. Official defaults: **Production** `https://agent.pay402.me` (Mainnet) · **Preview** `https://preview.agent.pay402.me` (Devnet). Run **`curl -sS "$BASE/api/v1/facilitator/health" | jq .solanaNetwork`** to confirm the cluster.

---

## Step 1 — Confirm the facilitator supports your scheme

```bash
curl -sS "$BASE/api/v1/facilitator/capabilities" | jq '.features'
```

Check `unsignedExactPaymentTxBuild: true` (for `exact`) or `unsignedSlaEscrowPaymentTxBuild: true` (for `sla-escrow`).

## Step 2 — Save the 402 response

From the seller's 402 body, save:
- **`accepts[]`** — pick one line matching your wallet's chain/asset
- **`resource`** — the resource descriptor

## Step 3 — Build an unsigned transaction

### For `exact` (instant payment):
```bash
curl -sS -X POST "$BASE/api/v1/facilitator/build-exact-payment-tx" \
  -H "Content-Type: application/json" \
  -d '{
    "payer": "<YOUR_PUBKEY>",
    "accepted": <THE_ACCEPTS_LINE_YOU_CHOSE>,
    "resource": <RESOURCE_FROM_402>
  }'
```

### For `sla-escrow` (time-bound escrow):
```bash
curl -sS -X POST "$BASE/api/v1/facilitator/build-sla-escrow-payment-tx" \
  -H "Content-Type: application/json" \
  -d '{
    "payer": "<YOUR_PUBKEY>",
    "accepted": <THE_ACCEPTS_LINE_YOU_CHOSE>,
    "resource": <RESOURCE_FROM_402>,
    "slaHash": "<64_HEX_CHARS>",
    "oracleAuthority": "<ORACLE_PUBKEY>"
  }'
```

**Response** gives you `transaction` (base64 unsigned tx) and `verifyBodyTemplate` (pre-filled verify/settle body).

## Step 4 — Sign the transaction

1. Decode `transaction` → base64 → bincode → `VersionedTransaction`
2. Sign at index `payerSignatureIndex` (from response) with your Solana keypair
3. Re-encode: `VersionedTransaction` → bincode → base64
4. Replace `verifyBodyTemplate.paymentPayload.payload.transaction` with the signed base64

## Step 5 — Verify then settle

```bash
# Verify
curl -sS -X POST "$BASE/api/v1/facilitator/verify" \
  -H "Content-Type: application/json" \
  -d @verify-body.json | jq .

# Settle (same body)
curl -sS -X POST "$BASE/api/v1/facilitator/settle" \
  -H "Content-Type: application/json" \
  -d @verify-body.json | jq .
```

**Success:** `{ "success": true, "transaction": "<SOLANA_SIGNATURE>" }`

## Step 6 — Access the paid resource

Send the **filled `verifyBodyTemplate`** JSON to the seller in the `PAYMENT-SIGNATURE` header:

```bash
curl -sS "<SELLER_RESOURCE_URL>" \
  -H "PAYMENT-SIGNATURE: $(cat verify-body.json)"
```

The seller forwards it to `/settle` and serves you the resource.

---

**If anything fails with blockhash errors, go back to Step 3** — Solana blockhashes expire in ~60 seconds.

**Full reference:** `GET /openapi.json` and `GET /agent-integration.md` on the facilitator.
