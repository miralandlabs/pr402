---
title: "Integrate your API"
---

# Integrate your API

**Audience:** Any developer with an existing web API (REST, GraphQL, etc.) who wants to gate routes behind paid access using the x402 protocol and the pr402 facilitator.

**Time to integrate:** ~30 minutes. No blockchain SDK required in your server.

> **New seller?** Complete [Start here · Sellers](/start-here.html) first (prerequisites + pick `exact` vs `sla-escrow`). **In a hurry?** [Quick reference · 5 steps](/quickstart-seller.html) (`exact` rail cheat-sheet).

> **Status.** pr402 is live on **Solana Mainnet** (`https://ipay.sh`) and **Devnet** (`https://preview.ipay.sh`); same service also served on `https://agent.pay402.me` / `https://preview.agent.pay402.me` (not deprecated). Behavior, feature flags, and fee parameters can evolve — treat **`GET /capabilities`** and **`GET /openapi.json`** on the host you actually call as the live contract.

Throughout this doc, replace **`$BASE`** with your facilitator origin — the same URL buyers use. Confirm **`solanaNetwork`** with **`GET $BASE/api/v1/facilitator/health`**. Target **Mainnet** (`https://ipay.sh`) for production; use **`https://preview.ipay.sh`** only if you want a Devnet rehearsal first.

---

## Two rails (pick one)

| | **`exact`** (UniversalSettle) | **`sla-escrow`** (SLA-Escrow) |
|---|---|---|
| **Settlement** | Instant — buyer pays, you deliver | Funds held in on-chain escrow until terms met or oracle verdict |
| **Best for** | API calls, instant access (~5¢+ per call) | High-value or slow fulfillment (shipping, custom work, SLAs) |
| **Integration size** | Smaller (402 + settle) | Larger (SLA terms, oracle, fulfillment) |

Most sellers start with **`exact`**. For **`sla-escrow`**, read the [Onboarding guide](/onboarding_guide.html) first.

Pricing and fee floors: [Start here · Appendix A · Protocol fees](/start-here.html#appendix-a-protocol-fees--pricing).

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

## Step 1 — Get your `payTo` (Preview / Activate)

Your buyers must pay into a **program PDA** (`payTo`), not your bare wallet.

1. Open **[ipay.sh](https://ipay.sh)** (Mainnet) or **[preview.ipay.sh](https://preview.ipay.sh)** (Devnet rehearsal).
2. Scroll to **§ seller lifecycle** (or `https://ipay.sh#seller-lifecycle`).
3. Paste your **seller pubkey** (or connect wallet).
4. Run **Preview** — note the vault / `payTo` the page shows (no on-chain change).

For `exact`, you can also resolve via:

```bash
export BASE="https://ipay.sh"   # Mainnet

curl -sS "$BASE/api/v1/facilitator/sellers/YOUR_PUBKEY/rails/exact" | jq .
```

**Recommended for Mainnet `exact`:** run **Activate** on the same site. Your wallet signs one provisioning transaction (~0.1 SOL for rent + fees). That makes you **sovereign** (**90 bps** protocol fee on every later payment). Skipping Activate is fine — pr402 JIT-provisions on first settle at **100 bps**. See [Appendix · Protocol fees](#supplemental-protocol-fees--pricing) or [Start here · Appendix A](/start-here.html#appendix-a-protocol-fees--pricing).

---

## Step 2 — Build your 402 payment body (once per product)

Do **not** hand-craft `extra` fields. POST a minimal draft to **`/payment-required/enrich`** once and **save the response**.

```bash
export BASE="https://ipay.sh"   # Mainnet

curl -sS -X POST "$BASE/api/v1/facilitator/payment-required/enrich" \
  -H "Content-Type: application/json" \
  -d '{
    "x402Version": 2,
    "resource": { "url": "https://your-api.com/premium-endpoint" },
    "accepts": [{
      "scheme": "exact",
      "network": "<NETWORK_FROM_/capabilities>",
      "payTo": "YOUR_WALLET_PUBKEY",
      "asset": "<USDC_MINT_FROM_/capabilities>",
      "amount": "50000",
      "maxTimeoutSeconds": 300
    }]
  }' | jq . > payment-body.json
```

Copy `network` and `asset` from **`GET $BASE/api/v1/facilitator/capabilities`** — do not paste Mainnet mints into a Devnet rehearsal (or vice versa).

Store `payment-body.json` as your **402 payment body**. Re-run `/payment-required/enrich` only when price, mint, network, or facilitator URL changes.

More detail on the `/payment-required/enrich` shortcut: [Quick reference · Steps 1–2](/quickstart-seller.html).

---

## Step 3 — The 3 changes to your code

### Change 1: Return HTTP 402 on unpaid requests

When a request arrives without a valid `PAYMENT-SIGNATURE` header, respond with **HTTP 402** and body = your **402 payment body** (from Step 2).

**Alternative:** build the body manually via `/sellers/{wallet}/rails/{scheme}` + `/supported` — see JSON shape below. **`/payment-required/enrich` is recommended** for most sellers.

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

> **Tip**: Copy `extra` from `GET /api/v1/facilitator/supported` → matching `kinds[]` entry + your wallet-specific fields. Or use **`POST /api/v1/facilitator/payment-required/enrich`** (Step 2).

---

### Change 2: Extract `PAYMENT-SIGNATURE` and settle via facilitator

When the buyer retries with proof, extract the header and POST it to the facilitator. pr402's `/settle` performs full verification internally before executing on-chain.

**Pseudocode — simple path (any language):**

```
function handle_paid_request(request):
    proof = request.headers["PAYMENT-SIGNATURE"]

    if proof is empty:
        return http_402(payment_body_json)

    payment_body = json_decode(proof)

    result = http_post(
        "$BASE/api/v1/facilitator/settle",
        headers: { "Content-Type": "application/json" },
        body: payment_body
    )

    if result.status != 200:
        return http_402(payment_body_json)

    return http_200(premium_content)
```

**Optional — verify-then-settle path (for audit linkage):**

```
function handle_paid_request(request):
    ...
    verify_result = http_post(".../verify", body: payment_body)
    if verify_result.status != 200:
        return http_402(payment_body_json)

    if verify_result.body.correlationId:
        payment_body.correlationId = verify_result.body.correlationId

    settle_result = http_post(".../settle", body: payment_body)
    ...
```

**curl equivalent:**

```bash
curl -sS -X POST "$BASE/api/v1/facilitator/settle" \
  -H "Content-Type: application/json" \
  -d "$DECODED_PAYMENT_SIGNATURE"
```

---

### Change 3: Return `PAYMENT-RESPONSE` header (v2)

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

## Language Examples {#language-examples}

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

## Step 4 — Test and go live

```bash
# Unpaid — expect 402
curl -sS -D - "https://your-api.com/premium-endpoint" -o /dev/null

# Paid path — use x402-seller-starter, a buyer agent, or @pr402/client
# See Buyer Quickstart if you need a payer
```

Run one real (small) Mainnet payment end-to-end before announcing the product.

**Optional Devnet rehearsal:** If you practiced on `preview.ipay.sh`, switch `$BASE` to `https://ipay.sh`, re-run **`/payment-required/enrich`** on Mainnet (mints and PDAs differ), update your stored 402 body, and test once more.

---

## Quick Reference

| What                          | Endpoint                                              | Method   | Notes                                                                                   |
| ----------------------------- | ----------------------------------------------------- | -------- | --------------------------------------------------------------------------------------- |
| Discover your `payTo` PDA     | `/api/v1/facilitator/sellers/X/rails/exact` | GET      |                                                                                         |
| Full onboard preview          | `/api/v1/facilitator/sellers/{X}/preview`              | GET      |                                                                                         |
| Upgrade naive 402             | `/api/v1/facilitator/payment-required/enrich`                         | POST     |                                                                                         |
| **Settle (verify + execute)** | `/api/v1/facilitator/settle`                      | **POST** | Verifies internally, then executes on-chain. Idempotent.                                |
| Verify (dry-run only)         | `/api/v1/facilitator/verify`                          | POST     | Optional pre-flight check. No on-chain cost. Returns `correlationId` for audit linkage. |
| Supported schemes/rails       | `/api/v1/facilitator/supported`                       | GET      |                                                                                         |
| Full discovery bundle         | `/api/v1/facilitator/capabilities`                    | GET      |                                                                                         |

> **pr402 vs standard x402 settle model**: In the generic x402 spec, `/verify` and `/settle` are separate steps with resource delivery in between. On Solana, blockhashes expire in ~60 seconds, making that gap risky. pr402's `/settle` runs verification internally before executing — so calling `/settle` alone is safe and sufficient. `/verify` remains useful as a zero-cost pre-flight check or to obtain a `correlationId` for DB audit trails.

**Canonical API spec:** `GET /openapi.json` on your facilitator deployment (see [API reference](/api-reference) for how humans and agents should use it).

**Full integration runbook:** [Agent integration](/agent-integration) here, or `GET /agent-integration.md` on the facilitator.

**Reference implementation:** [x402-seller-starter](https://github.com/miraland-labs/x402-seller-starter) (Rust + Axum).

---

## Supplemental · Protocol fees & pricing

> Same tables as [Start here · Appendix A](/start-here.html#appendix-a-protocol-fees--pricing). Kept here for sellers pricing while integrating.

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

- **`exact`:** aim for **≥ ~$0.05 USDC** per call. Below **~$0.02**, more than half of revenue can go to protocol fees.
- **`sla-escrow`:** aim for **≥ ~$10 USDC** per payment. Smaller tickets → use **`exact`**.

**Sovereign discount:** Self-provision via **Activate** (~**0.1 SOL** one-time) drops `exact` protocol fee from 100 bps → 90 bps.

**Facilitator comparison (CDP, x402.org, `pay` CLI):** [Start here · Appendix B](/start-here.html#appendix-b-why-pr402-vs-other-facilitators) · [Choosing x402 on Solana](/pr402-vs-alternatives.html)
