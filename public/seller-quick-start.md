# Seller Quick Start: Monetize Your API with x402

**Audience**: Any developer with an existing web API (REST, GraphQL, etc.) who wants to gate routes behind paid access using the x402 protocol and the pr402 facilitator.

**Time to integrate**: ~30 minutes. No blockchain SDK required in your server.

> **Seller documentation map.** This is the canonical seller guide. Other seller-facing pages exist for good reasons; use them in this order:
>
> | When you want… | Read |
> |---|---|
> | A 30-minute walkthrough with language examples (Rust / Python / JS / Go) | **This page** |
> | A 5-step cheat-sheet after you already know the flow | [Seller shortcut · 5 steps](/quickstart-seller.md) |
> | Deep dive on sovereign fees, JIT provisioning, one-asset-per-wallet policy | [Onboarding guide](/onboarding_guide.md) |
> | The Preview → Activate → Verify lifecycle and how each HTTP endpoint maps | [Agent integration · Seller agents](/agent-integration.md#seller-agents-resource-providers) |
> | Machine-readable contract | `GET /openapi.json` on the host you call |

> **Status.** pr402 is live on **Solana Mainnet** (`https://ipay.sh`) and **Devnet** (`https://preview.ipay.sh`); same service also served on `https://agent.pay402.me` / `https://preview.agent.pay402.me` (not deprecated). Behavior, feature flags, and fee parameters can evolve — treat **`GET /capabilities`** and **`GET /openapi.json`** on the host you actually call as the live contract.

Throughout this doc, replace **`$BASE`** with your facilitator origin — the same URL buyers use. Confirm **`solanaNetwork`** with **`GET $BASE/api/v1/facilitator/health`**.

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
     | PAYMENT-SIGNATURE: <base64 JSON>                       |
     |                         |--- POST /settle ------------>|
     |                         |   (verify + execute on-chain)|
     |                         |<-- 200 OK (settled) ---------|
     |<-- 200 + content -------|                              |
     |   PAYMENT-RESPONSE: {…} |                              |
```

> `/settle` performs verification internally — calling it alone is the simplest integration.
> For audit linkage, send your own `X-Correlation-ID` directly to `/settle`.

**Key insight**: Your server never touches Solana directly. You return a 402, extract the payment proof header, and forward it to the facilitator. That's it.

> **pr402 settlement model (Solana-specific)**: The standard x402 flow is: `/verify` → deliver resource → `/settle`. On Solana, signed transactions contain a blockhash that expires in ~60 seconds. If resource delivery takes any real time between verify and settle, the blockhash expires and settlement fails. In pr402, `**/settle` already performs verification internally** before executing on-chain — so calling `/settle` alone is sufficient and safe. It is also idempotent: if the transaction is already confirmed on-chain, it returns success.
>
> **When is `/verify` still useful?** Only as a diagnostic pre-flight. Its success is temporary and does not prove payment. Production sellers should call `/settle` immediately; pass their own `X-Correlation-ID` when audit linkage is needed.

---

## The 3 Changes to Your Code

### Change 1: Return HTTP 402 on Unpaid Requests

When a request arrives without a valid `PAYMENT-SIGNATURE` header, respond with **HTTP 402** and a JSON body describing what to pay.

**What you need first** — look up your vault PDA (one-time):

```bash
curl -sS "$BASE/api/v1/facilitator/sellers/YOUR_PUBKEY/rails/exact" | jq .
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

> **Tip**: Copy `extra` from `GET /api/v1/facilitator/supported` → matching `kinds[]` entry + your wallet-specific fields. Or use the **`/payment-required/enrich`** endpoint to have the facilitator build this for you (see below).

---

### Change 2: Extract `PAYMENT-SIGNATURE` and Settle via Facilitator

When the buyer retries with proof, decode the header and POST it to the facilitator. pr402's `/settle` performs full verification internally before executing on-chain, so calling `/settle` alone is the production path. Send your own `X-Correlation-ID` header when audit linkage is needed.

**Pseudocode — simple path (any language):**

```
function handle_paid_request(request):
    proof = request.headers["PAYMENT-SIGNATURE"]

    if proof is empty:
        return http_402(accepts_json)

    # Official clients emit base64 JSON; raw JSON remains accepted for compatibility.
    payment_body = json_decode(base64_decode(proof))
        or_else json_decode(proof)

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

**Diagnostic only — standalone `/verify`:**

```
function handle_paid_request(request):
    ...
    # Step 1: dry-run verification (no on-chain cost)
    verify_result = http_post(".../verify", body: payment_body)
    if verify_result.status != 200:
        return http_402(accepts_json)

    # Do not deliver here. If continuing, settle immediately with the same proof.
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
use base64::{engine::general_purpose::STANDARD, Engine as _};

let proof = extract_payment_header_value(&headers);
match proof {
    Some(value) => {
        let decoded = STANDARD.decode(&value).ok().and_then(|v| String::from_utf8(v).ok());
        let body: serde_json::Value = serde_json::from_str(decoded.as_deref().unwrap_or(&value))?;
        let result = http_client
            .post(format!("{FACILITATOR_URL}/api/v1/facilitator/settle"))
            .json(&body)
            .send()
            .await?;
        // Return 200 with PAYMENT-RESPONSE header
    }
    None => {
        // Return 402 with accepts[] body
    }
}
```

### Python (Flask / FastAPI)

```python
import base64
import json

proof = request.headers.get("PAYMENT-SIGNATURE")
if not proof:
    return JSONResponse(status_code=402, content=accepts_body)

try:
    payment_body = json.loads(base64.b64decode(proof, validate=True))
except (ValueError, json.JSONDecodeError):
    payment_body = json.loads(proof)

import httpx
result = httpx.post(f"{FACILITATOR_URL}/api/v1/facilitator/settle",
                    json=payment_body)
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

let paymentBody;
try {
  paymentBody = JSON.parse(Buffer.from(proof, 'base64').toString('utf8'));
} catch {
  paymentBody = JSON.parse(proof);
}

const result = await fetch(`${FACILITATOR_URL}/api/v1/facilitator/settle`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(paymentBody),
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

paymentBody, err := base64.StdEncoding.DecodeString(proof)
if err != nil || !json.Valid(paymentBody) {
    paymentBody = []byte(proof)
}
if !json.Valid(paymentBody) {
    http.Error(w, "invalid PAYMENT-SIGNATURE", http.StatusBadRequest)
    return
}

resp, err := http.Post(facilitatorURL+"/api/v1/facilitator/settle",
    "application/json", bytes.NewReader(paymentBody))
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

## Shortcut: The `/payment-required/enrich` Endpoint

Don't want to look up vault PDAs or merge `extra` fields? Post a minimal 402 body to `**POST /api/v1/facilitator/payment-required/enrich**` and get back a fully institutional response.

```bash
# Your naive 402 body (bare wallet as payTo):
curl -X POST "$BASE/api/v1/facilitator/payment-required/enrich" \
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
| Discover your `payTo` PDA     | `/api/v1/facilitator/sellers/X/rails/exact` | GET      |                                                                                         |
| Full onboard preview          | `/api/v1/facilitator/sellers/{X}/preview`              | GET      |                                                                                         |
| Upgrade naive 402             | `/api/v1/facilitator/payment-required/enrich`                         | POST     |                                                                                         |
| **Settle (verify + execute)** | `/api/v1/facilitator/settle`                          | **POST** | Verifies internally, then executes on-chain. Idempotent.                                |
| Verify (dry-run only)         | `/api/v1/facilitator/verify`                          | POST     | Diagnostic pre-flight only; success is not payment.                                    |
| Supported schemes/rails       | `/api/v1/facilitator/supported`                       | GET      |                                                                                         |
| Full discovery bundle         | `/api/v1/facilitator/capabilities`                    | GET      |                                                                                         |


> **pr402 vs standard x402 settle model**: In the generic x402 spec, `/verify` and `/settle` are separate steps with resource delivery in between. On Solana, blockhashes expire quickly, making that gap risky. pr402's `/settle` verifies, submits, and confirms in one call. Deliver only after it succeeds. `/verify` is diagnostic only; send `X-Correlation-ID` directly to `/settle` for audit linkage.

**Canonical API spec**: `GET /openapi.json` on your facilitator deployment.
**Full integration runbook**: `GET /agent-integration.md` on your facilitator deployment.
**Reference implementation**: [x402-seller-starter](https://github.com/miraland-labs/x402-seller-starter) (Rust + Axum).
