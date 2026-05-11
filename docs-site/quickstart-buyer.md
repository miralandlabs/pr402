# Buyer Quick Start — 6 Steps to Pay via x402

> **You have a Solana wallet and received HTTP 402 from a seller. Here's what to do.**

> **Status.** pr402 and the `exact` rail are live on **Solana Mainnet** (`https://ipay.sh`) and **Devnet** (`https://preview.ipay.sh`); same service also served on `https://agent.pay402.me` / `https://preview.agent.pay402.me` (not deprecated). For `sla-escrow`, the `oracle_authority` on the payment is chosen by the seller — trust that authority explicitly before funding.

## Install {#install}

**Fastest path — use the published SDK.** One command does the whole loop below.

```bash
# Node (≥ 18)
npm i -g @pr402/client
pr402-buy --resource <URL> --payer <keypair.json> --mint <MINT>

# Rust (1.75+)
cargo install pr402-client
pr402-buy --resource <URL> --payer <keypair.json> --mint <MINT>

# Zero-install (Node)
npx @pr402/client pr402-buy --resource <URL> --payer <keypair.json> --mint <MINT>
```

Both packages also expose a library — `X402AgentClient` — for embedding in long-running agents. See the package READMEs ([npm](https://www.npmjs.com/package/@pr402/client), [crates.io](https://crates.io/crates/pr402-client)).

The six steps below document the protocol underneath, for implementing from scratch in other languages or verifying the SDK's behavior.

---

Replace **`$BASE`** with the facilitator URL the seller documents. Run **`curl -sS "$BASE/api/v1/facilitator/health" | jq .solanaNetwork`** to confirm the cluster.

Do not silently substitute a different facilitator origin than the seller documented. `payTo`, program ids, asset allowlists, and oracle authorities are deployment-specific.

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

**Response** gives you `transaction` (base64 unsigned tx) and `verifyBodyTemplate` (pre-filled verify/settle body). If your 402 line uses **`v2:solana:exact`** or **`v2:solana:sla-escrow`**, the template’s **`scheme`** fields are normalized to wire **`exact`** / **`sla-escrow`** — use them as returned.

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

**Full reference:** [API overview](/api-reference) · `{FACILITATOR}/openapi.json` · [Agent integration](/agent-integration) (or `GET /agent-integration.md` on the facilitator).

## Buyer launch checklist

- Cache `GET /capabilities` per facilitator host and invalidate it on failed build/verify responses.
- Treat `verifyBodyTemplate` as authoritative; only replace the transaction payload after signing.
- Record `correlationId` from `/verify` and reuse it on `/settle` when present.
- For `sla-escrow`, verify the oracle authority and profile id before funding.
