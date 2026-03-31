# Agent integration (pr402 facilitator)

Short runbook for **buyer-side** agents (wallets, OpenClaw-style orchestrators, or custom HTTP clients). **Canonical contract:** OpenAPI 3.1 at **`GET /openapi.json`** on your facilitator base URL (e.g. `https://preview.pr402.signer-payer.me/openapi.json`). **This file** lives in **`public/agent-integration.md`** in the repo and is served like **`openapi.json`** (static deploy; see `capabilities.httpEndpoints.agentIntegration`). Machine discovery: **`GET /api/v1/facilitator/capabilities`** includes `httpEndpoints` and `openApi`.

## 1. Discover

```bash
BASE="https://preview.pr402.signer-payer.me"
curl -sS "$BASE/api/v1/facilitator/supported" | jq .
# or
curl -sS "$BASE/api/v1/facilitator/capabilities" | jq .
```

Pick one `kinds[]` entry that matches the **402 `accepts[]`** line your resource returned (`scheme`, `network`, `asset`, `amount`, `payTo`, `extra`).

## 2. Build unsigned tx (when the RP uses pr402 builds)

Two build endpoints; **do not** confuse them.

| `accepts[].scheme` | Endpoint | Who signs before verify |
|--------------------|----------|-------------------------|
| `exact` | `POST /api/v1/facilitator/build-exact-payment-tx` | Payer signs token authority; facilitator is fee payer at settle (default). |
| `sla-escrow` | `POST /api/v1/facilitator/build-sla-escrow-payment-tx` | Buyer partial sign; facilitator completes fee payer at settle (default). |

Request bodies are in **`openapi.json`** (`BuildExactPaymentTxRequest`, `BuildSlaEscrowPaymentTxRequest`). Response includes **`verifyBodyTemplate`** and base64 **`transaction`** (bincode `VersionedTransaction`, unsigned).

## 3. Sign locally

Use your stack’s Solana signer. Replace **`paymentPayload.payload.transaction`** in `verifyBodyTemplate` with the **signed** tx base64. Keep **`accepted`** identical to **`paymentRequirements`**.

If **BlockhashNotFound** appears on settle, call **build** again, re-sign, then verify/settle again.

## 4. Verify and settle

Use the **same** JSON body for both calls. Optional: set **`correlationId`** in the body and/or **``** header so Postgres audit merges one row.

```bash
curl -sS -X POST "$BASE/api/v1/facilitator/verify" \
  -H "Content-Type: application/json" \
  -d @verify-body.json | jq .

curl -sS -X POST "$BASE/api/v1/facilitator/settle" \
  -H "Content-Type: application/json" \
  -d @verify-body.json | jq .
```

## 5. When the RP does not use build APIs

Some flows (e.g. SLA **buyer-paid** CLI `fund-payment`) already have a fully signed fund tx in the proof. Still use the same **`POST /verify`** and **`POST /settle`** body shape (x402 v2); the payload carries the signed transaction from the RP’s 402 flow.

## TypeScript helpers

Repo-local thin **`fetch`** wrappers: [`sdk/facilitator-build-tx.ts`](../sdk/facilitator-build-tx.ts) — `getCapabilities`, `getSupported`, `verifyPayment`, `settlePayment`, `buildExactPaymentTx`, `buildSlaEscrowPaymentTx`, `fetchFacilitatorOpenApi`.

## Rust helpers

Enable Cargo feature **`facilitator-http`** on the **`pr402`** library dependency, then use **`pr402::sdk::http`**:

- **Typed client:** `FacilitatorHttpClient::new(base_url)?` → `.capabilities()`, `.build_exact_payment_tx(&req)`, `.verify_payment(&json, correlation_id)`, etc.
- **Free functions** (TS parity): `fetch_facilitator_openapi`, `get_supported`, `verify_payment`, `build_exact_payment_tx`, …
- **Extras:** `fetch_agent_integration_markdown` (`GET /agent-integration.md`), `normalize_base_url`.

Paths and JSON shapes match [`sdk/facilitator-build-tx.ts`](../sdk/facilitator-build-tx.ts); **`facilitator`** stays on default features (no `reqwest` in the serverless binary).

## Specs

- x402 v2: [x402-specification-v2.md](https://github.com/coinbase/x402/blob/main/specs/x402-specification-v2.md)
- Facilitator HTTP: **`/openapi.json`** and Markdown runbook **`/agent-integration.md`** on the deployment
