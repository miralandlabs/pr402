# Buyer Quick Start

> You have a Solana wallet and got **HTTP 402** from a seller.

> **Status.** Live on Mainnet (`https://ipay.sh`) and Devnet (`https://preview.ipay.sh`). Same APIs at `https://agent.pay402.me` / `https://preview.agent.pay402.me`.

Set **`$BASE`** to the facilitator URL the **seller documents**. Confirm cluster: `GET $BASE/api/v1/facilitator/health`.

Do not substitute a different facilitator origin. `payTo`, mint allowlists, and oracle authorities are deployment-specific.

---

## Default — use the SDK

```bash
npm i -g @pr402/client
pr402-buy --resource <URL> --payer <keypair.json> --mint <MINT>
```

One-shot: `npx @pr402/client pr402-buy ...` · Rust: `cargo install pr402-client`

**Library:** `X402AgentClient.fetchWithAutoPay` · **Starter:** [x402-buyer-starter](https://github.com/miraland-labs/x402-buyer-starter) (`createPay402Fetch`, Bash/Python demos) · **MCP:** `npx -y @pr402/mcp-server` → `pr402_pay_http_resource`

**Flow:** 402 → build → sign → retry the seller with **`PAYMENT-SIGNATURE`**. The **seller** calls facilitator **`/settle`**. Do **not** call **`/verify`** or **`/settle`** as the buyer first.

---

## Manual path (`exact` rail)

For languages without a published SDK:

1. **402** — Save one **`accepts[]`** line and **`resource`**.
2. **Build** — `POST $BASE/api/v1/facilitator/build-exact-payment-tx` with `{ payer, accepted, resource }`. If the 402 line says `v2:solana:exact`, send wire **`exact`** on the request.
3. **Sign** — Sign `transaction` at **`payerSignatureIndex`**. Put signed base64 into **`verifyBodyTemplate.paymentPayload.payload.transaction`**.
4. **Retry seller** — Same request with **`PAYMENT-SIGNATURE`** (raw JSON or base64). Seller settles and returns **200** + optional **`PAYMENT-RESPONSE`**.

Blockhash expired? Rebuild from step 2.

### Build request (step 2)

```bash
curl -sS -X POST "$BASE/api/v1/facilitator/build-exact-payment-tx" \
  -H "Content-Type: application/json" \
  -d '{
    "payer": "<YOUR_PUBKEY>",
    "accepted": <THE_ACCEPTS_LINE_YOU_CHOSE>,
    "resource": <RESOURCE_FROM_402>
  }'
```

### Retry (step 4)

```bash
curl -sS "<SELLER_RESOURCE_URL>" \
  -H "PAYMENT-SIGNATURE: $(cat verify-body.json)"
```

### Optional — find sellers first

```bash
curl -sS "$BASE/api/v1/facilitator/providers?limit=50" | jq '.entries[] | {displayName, serviceUrl, tags}'
```

### `sla-escrow`

Use **`POST /build-sla-escrow-payment-tx`**. See [Agent integration](/agent-integration) and the [oracles Buyer Guide](https://github.com/miraland-labs/oracles/blob/main/docs/BUYER_GUIDE.md).

---

## Advanced — buyer-side `/verify` + `/settle`

For debugging or custom flows **without** a seller gate. **Not** the default for `pr402-buy`, MCP, or x402-buyer-starter.

```bash
curl -sS -X POST "$BASE/api/v1/facilitator/verify" \
  -H "Content-Type: application/json" -d @verify-body.json

curl -sS -X POST "$BASE/api/v1/facilitator/settle" \
  -H "Content-Type: application/json" -d @verify-body.json
```

`/settle` is idempotent. If the seller still requires proof, retry with **`PAYMENT-SIGNATURE`** as in the manual path.

---

## Checklist

- Match the seller's facilitator host (preview vs mainnet).
- After build, only replace the signed **`transaction`** in **`verifyBodyTemplate`**.
- For **`sla-escrow`**, verify the seller's oracle authority before funding.

**More:** [Agent integration](/agent-integration) · **`GET /openapi.json`**
