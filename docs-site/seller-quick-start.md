---
title: "Seller Quick Start: Monetize Your API with x402"
---

# Seller Quick Start: Monetize Your API with x402

**Audience**: Any developer with an existing web API (REST, GraphQL, etc.) who wants to gate routes behind paid access using the x402 protocol and the pr402 facilitator.

**Time to integrate**: ~30 minutes. No blockchain SDK required in your server.

**Prefer a five-step cheat-sheet?** Use [Seller shortcut · 5 steps](/quickstart-seller.html) for the minimal `/upgrade` path.

> **Seller documentation map.** This is the canonical seller guide. Other seller-facing pages exist for good reasons; use them in this order:
>
> | When you want… | Read |
> |---|---|
> | Prerequisites, fees, pick exact vs sla-escrow, numbered checklist | [Start here · Seller checklist](/start-here.html) |
> | A 30-minute walkthrough with language examples (Rust / Python / JS / Go) | **This page** |
> | A 5-step cheat-sheet after you already know the flow | [Seller shortcut · 5 steps](/quickstart-seller.html) |
> | Deep dive on sovereign fees, JIT provisioning, one-asset-per-wallet policy | [Onboarding guide](/onboarding_guide.html) |
> | The Preview → Activate → Verify lifecycle and how each HTTP endpoint maps | [Agent integration · Seller agents](/agent-integration.html#seller-agents-resource-providers) |
> | Machine-readable contract | `GET /openapi.json` on the host you call |

> **Status.** pr402 is live on **Solana Mainnet** (`https://ipay.sh`) and **Devnet** (`https://preview.ipay.sh`); same service also served on `https://agent.pay402.me` / `https://preview.agent.pay402.me` (not deprecated). Behavior, feature flags, and fee parameters can evolve — treat **`GET /capabilities`** and **`GET /openapi.json`** on the host you actually call as the live contract.

Throughout this doc, replace **`$BASE`** with your facilitator origin — the same URL buyers use. Confirm **`solanaNetwork`** with **`GET $BASE/api/v1/facilitator/health`**. Target **Mainnet** (`https://ipay.sh`) for production; use **`https://preview.ipay.sh`** only if you want a Devnet rehearsal first ([Start here](/start-here.html#why-pr402-true-differentiators)).

> **Why pr402?** Paired **Mainnet ↔ Devnet** hosts, **`sla-escrow`** (not on CDP/x402.org Solana), sovereign fee tier, `/upgrade` without PDA math. [Choosing x402 on Solana](/pr402-vs-alternatives.html) · [Short differentiators](/start-here.html#why-pr402-true-differentiators).

---

## Two rails · why `sla-escrow` matters for buyers

| | **`exact`** (UniversalSettle) | **`sla-escrow`** (SLA-Escrow) |
|---|---|---|
| **Settlement** | Instant — buyer pays, you deliver | Funds held in on-chain escrow until terms met or oracle verdict |
| **Best for** | API calls, instant access (~5¢+ per call) | High-value or slow fulfillment (shipping, custom work, SLAs) |
| **Buyer protection** | Standard x402 instant pay model | **Escrow + oracle** — refund/release paths enforced on-chain |
| **Elsewhere in x402** | Common among facilitators | **pr402-only today** — no standard x402 facilitator offers equivalent escrow protection |

Most sellers start with **`exact`**. Offer **`sla-escrow`** when buyers need assurance that payment is not released until delivery — that buyer trust is a seller differentiator, not just extra integration work.

---

## Protocol fees & pricing

Treat **`GET $BASE/api/v1/facilitator/capabilities`** as authoritative. Snapshot for the live **ipay.sh** deployment:

| | **`exact`** | **`sla-escrow`** |
|---|---|---|
| **Standard protocol fee** | **100 bps** (1.00%) | **100 bps** (1.00%) |
| **Sovereign protocol fee** | **90 bps** (0.90%) after **Activate** on [ipay.sh](https://ipay.sh) | — |
| **Minimum protocol fee (USDC)** | **$0.01** | **$0.10** |
| **Oracle tip** | none | **100 bps** on verdict (no floor) |

**`exact` floor math (USDC):** fee = max(1% × amount, $0.01).

| Price per call | Protocol fee | Fee as % of revenue |
|---|---|---|
| $0.01 | $0.01 | 100% |
| $0.02 | $0.01 | 50% |
| $0.05 | $0.01 | 20% |
| $0.10+ | scales with 1% | ≤ 10% and falling |

**Draft pricing guidance:**

- **`exact`:** aim for **≥ ~$0.05 USDC** per call. Below **~$0.02**, more than half of revenue can go to protocol fees. pr402 does not fully subsidize tx gas like some large facilitators; the **1 cent** floor covers operating cost.
- **`sla-escrow`:** aim for **≥ ~$10 USDC** per payment. Smaller tickets → use **`exact`**.

**Sovereign discount:** Self-provision via **Activate** (~**0.1 SOL** one-time) drops `exact` protocol fee from 100 bps → 90 bps. Skip Activate and pr402 **JIT-provisions** on first settle at 100 bps — your choice.

Full checklist: [Start here · Seller checklist](/start-here.html).

---

## How It Works (30-Second Overview)

```
Buyer Agent              Your API Server              pr402 Facilitator
     |                         |                              |
     |--- GET /api/premium --->|                              |
     |<-- 402 + accepts[] -----|                              |
     |                         |                              |
     |--- build tx ------------------------------------------>|
     |<-- unsigned tx + verifyBodyTemplate -------------------|
     |                         |                              |
     |   (sign locally)        |                              |
     |                         |                              |
     |--- GET /api/premium --->|                              |
     |   PAYMENT-SIGNATURE: {…}|                              |
     |                         |--- POST /settle ------------>|
     |                         |   (verify + execute on-chain)|
     |                         |<-- 200 OK (settled) ---------|
     |<-- 200 + content -------|                              |
     |   PAYMENT-RESPONSE: {…} |                              |
```

> `/settle` performs verification internally — calling it alone is the simplest integration.
> For audit linkage, optionally call `/verify` first to obtain a `correlationId`.

**Key insight**: Your server never touches Solana directly. You return a 402, extract the payment proof header, and forward it to the facilitator. That's it.

> **pr402 settlement model (Solana-specific)**: The standard x402 flow is: `/verify` → deliver resource → `/settle`. On Solana, signed transactions contain a blockhash that expires in ~60 seconds. If resource delivery takes any real time between verify and settle, the blockhash expires and settlement fails. In pr402, **`POST /api/v1/facilitator/settle`** already performs verification internally before executing on-chain — so calling `/settle` alone is sufficient and safe. It is also idempotent: if the transaction is already confirmed on-chain, it returns success.
>
> **When is `/verify` still useful?** As a **pre-flight dry-run**: it validates the proof (signature, amounts, recipient, mint) without spending any Solana fees. Useful for diagnostics, or if your seller-side logic needs to confirm validity before committing business logic. The `x402-seller-starter` reference implementation calls both (`/verify` → `/settle`) to obtain a `correlationId` for audit linkage before settling.

---

## The 3 Changes to Your Code

### Change 1: Return HTTP 402 on Unpaid Requests

When a request arrives without a valid `PAYMENT-SIGNATURE` header, respond with **HTTP 402** and a JSON body describing what to pay.

**What you need first** — look up your vault PDA (one-time):

```bash
curl -sS "$BASE/api/v1/facilitator/discovery?wallet=YOUR_PUBKEY&scheme=exact" | jq .
# → Note the vaultPda value — that becomes your payTo
```

**Your 402 response body** (x402 v2 format):

```json
{
  "x402Version": 2,
  "accepts": [
    {
      "scheme": "exact",
      "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
      "asset": "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
      "amount": "50000",
      "payTo": "<YOUR_VAULT_PDA>",
      "maxTimeoutSeconds": 300,
      "extra": {
        "feePayer": "...",
        "programId": "...",
        "configAddress": "...",
        "feeBps": "100",
        "merchantWallet": "<YOUR_ACTUAL_WALLET>"
      }
    }
  ],
  "error": "Payment Required",
  "description": "Pay 0.05 USDC to access this endpoint"
}
```

> **Tip**: Copy `extra` from `GET /api/v1/facilitator/supported` → matching `kinds[]` entry + your wallet-specific fields. Or use **`POST /api/v1/facilitator/upgrade`** to have the facilitator build this for you (see below).

---

### Change 2: Extract `PAYMENT-SIGNATURE` and Settle via Facilitator

When the buyer retries with proof, extract the header and POST it to the facilitator. pr402's `/settle` performs full verification internally before executing on-chain, so calling `/settle` alone is the simplest path. For audit linkage, you can optionally call `/verify` first to obtain a `correlationId`, then pass it to `/settle`.

**Pseudocode — simple path (any language):**

```
function handle_paid_request(request):
    proof = request.headers["PAYMENT-SIGNATURE"]

    if proof is empty:
        return http_402(accepts_json)

    payment_body = json_decode(proof)

    # /settle verifies internally then executes on-chain.
    # Idempotent: already-confirmed transactions return success.
    result = http_post(
        "$BASE/api/v1/facilitator/settle",
        headers: { "Content-Type": "application/json" },
        body: payment_body
    )

    if result.status != 200:
        return http_402(accepts_json)

    # Payment confirmed — serve the premium content
    return http_200(premium_content)
```

**Optional — verify-then-settle path (for audit linkage):**

```
function handle_paid_request(request):
    ...
    # Step 1: dry-run verification (no on-chain cost)
    verify_result = http_post(".../verify", body: payment_body)
    if verify_result.status != 200:
        return http_402(accepts_json)

    # Step 2: carry correlationId into settle for DB audit trail
    if verify_result.body.correlationId:
        payment_body.correlationId = verify_result.body.correlationId

    # Step 3: settle (verifies again internally + executes on-chain)
    settle_result = http_post(".../settle", body: payment_body)
    ...
```

**curl equivalent** (what your server does internally):

```bash
curl -sS -X POST "$BASE/api/v1/facilitator/settle" \
  -H "Content-Type: application/json" \
  -d "$DECODED_PAYMENT_SIGNATURE"
```

---

### Change 3: Return `PAYMENT-RESPONSE` Header (v2)

After successful settlement, include the result in a `PAYMENT-RESPONSE` header so buyers can confirm finality.

```
function handle_paid_request(request):
    ...
    result = http_post(".../settle", body: payment_body)

    if result.status == 200:
        encoded = base64_encode(json_encode(result.body))
        response.headers["PAYMENT-RESPONSE"] = encoded
        return http_200(premium_content)
```

---

## Language Examples

### Rust (Axum)

See the complete working example in [x402-seller-starter](https://github.com/miraland-labs/x402-seller-starter):

```rust
let proof = extract_payment_header_value(&headers);
match proof {
    Some(value) => {
        let body: serde_json::Value = serde_json::from_str(&value)?;
        let result = facilitator.verify_and_settle(&body).await?;
        // Return 200 with PAYMENT-RESPONSE header
    }
    None => {
        // Return 402 with accepts[] body
    }
}
```

### Python (Flask / FastAPI)

```python
proof = request.headers.get("PAYMENT-SIGNATURE")
if not proof:
    return JSONResponse(status_code=402, content=accepts_body)

import httpx
result = httpx.post(f"{FACILITATOR_URL}/api/v1/facilitator/settle",
                    json=json.loads(proof))
if result.status_code != 200:
    return JSONResponse(status_code=402, content=accepts_body)

import base64
response = JSONResponse(content=premium_data)
response.headers["PAYMENT-RESPONSE"] = base64.b64encode(result.text.encode()).decode()
return response
```

### JavaScript / TypeScript (Express / Node)

```javascript
const proof = req.headers['payment-signature'];
if (!proof) {
  return res.status(402).json(acceptsBody);
}

const result = await fetch(`${FACILITATOR_URL}/api/v1/facilitator/settle`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: proof,
});
if (!result.ok) {
  return res.status(402).json(acceptsBody);
}

const settleResult = await result.text();
res.set('PAYMENT-RESPONSE', Buffer.from(settleResult).toString('base64'));
res.json(premiumContent);
```

### Go (net/http)

```go
proof := r.Header.Get("PAYMENT-SIGNATURE")
if proof == "" {
    w.WriteHeader(http.StatusPaymentRequired)
    json.NewEncoder(w).Encode(acceptsBody)
    return
}

resp, err := http.Post(facilitatorURL+"/api/v1/facilitator/settle",
    "application/json", strings.NewReader(proof))
if err != nil || resp.StatusCode != 200 {
    w.WriteHeader(http.StatusPaymentRequired)
    json.NewEncoder(w).Encode(acceptsBody)
    return
}

body, _ := io.ReadAll(resp.Body)
w.Header().Set("PAYMENT-RESPONSE", base64.StdEncoding.EncodeToString(body))
json.NewEncoder(w).Encode(premiumContent)
```

---

## Shortcut: The `/upgrade` Endpoint

Don't want to look up vault PDAs or merge `extra` fields? Post a minimal 402 body to **`POST /api/v1/facilitator/upgrade`** and get back a fully institutional response.

```bash
# Your naive 402 body (bare wallet as payTo):
curl -X POST "$BASE/api/v1/facilitator/upgrade" \
  -H "Content-Type: application/json" \
  -d '{
    "x402Version": 2,
    "accepts": [{
      "scheme": "exact",
      "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
      "payTo": "YOUR_BARE_WALLET",
      "amount": "50000",
      "asset": "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
    }]
  }'
# → Returns the same body with payTo replaced by vault PDA and extra injected
```

Cache the result and return it as your 402 response.

---

## Quick Reference


| What                          | Endpoint                                              | Method   | Notes                                                                                   |
| ----------------------------- | ----------------------------------------------------- | -------- | --------------------------------------------------------------------------------------- |
| Discover your `payTo` PDA     | `/api/v1/facilitator/discovery?wallet=X&scheme=exact` | GET      |                                                                                         |
| Full onboard preview          | `/api/v1/facilitator/onboard?wallet=X`                | GET      |                                                                                         |
| Upgrade naive 402             | `/api/v1/facilitator/upgrade`                         | POST     |                                                                                         |
| **Settle (verify + execute)** | `/api/v1/facilitator/settle`                      | **POST** | Verifies internally, then executes on-chain. Idempotent.                                |
| Verify (dry-run only)         | `/api/v1/facilitator/verify`                          | POST     | Optional pre-flight check. No on-chain cost. Returns `correlationId` for audit linkage. |
| Supported schemes/rails       | `/api/v1/facilitator/supported`                       | GET      |                                                                                         |
| Full discovery bundle         | `/api/v1/facilitator/capabilities`                    | GET      |                                                                                         |


> **pr402 vs standard x402 settle model**: In the generic x402 spec, `/verify` and `/settle` are separate steps with resource delivery in between. On Solana, blockhashes expire in ~60 seconds, making that gap risky. pr402's `/settle` runs verification internally before executing — so calling `/settle` alone is safe and sufficient. `/verify` remains useful as a zero-cost pre-flight check or to obtain a `correlationId` for DB audit trails.

**Canonical API spec:** `GET /openapi.json` on your facilitator deployment (see [API reference](/api-reference) for how humans and agents should use it).

**Full integration runbook:** [Agent integration](/agent-integration) here, or `GET /agent-integration.md` on the facilitator.

**Reference implementation:** [x402-seller-starter](https://github.com/miraland-labs/x402-seller-starter) (Rust + Axum).