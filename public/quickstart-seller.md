# Seller Quick Start — Monetize Your API with x402

> **You have a Solana wallet and want to charge for API calls. Here's the fastest path.**

> **Launch phase:** **Experimental** — **use at your own risk**.

Replace **`$BASE`** with your facilitator URL. **Recommended:** **Production** `https://ipay.sh` (Mainnet) · **Preview** `https://preview.ipay.sh` (Devnet). **Also:** `https://agent.pay402.me` / `https://preview.agent.pay402.me` (same service). Confirm **`solanaNetwork`** with **`GET /api/v1/facilitator/health`** on that host.

Use the same `$BASE` in your public docs, 402 bodies, buyer instructions, `/verify`, and `/settle`. Most integration failures come from mixing preview/mainnet origins or publishing a bare wallet where this facilitator expects a PDA.

---

## Step 1 — Build a naive 402 body with your wallet

```json
{
  "x402Version": 2,
  "resource": { "url": "https://your-api.com/premium-endpoint" },
  "accepts": [
    {
      "scheme": "exact",
      "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
      "payTo": "<YOUR_WALLET_PUBKEY>",
      "asset": "<USDC_MINT_PUBKEY>",
      "amount": "50000",
      "maxTimeoutSeconds": 300
    }
  ]
}
```

## Step 2 — Upgrade it to institutional format (one POST)

```bash
curl -sS -X POST "$BASE/api/v1/facilitator/upgrade" \
  -H "Content-Type: application/json" \
  -d '<YOUR_402_BODY>' | jq .
```

The facilitator turns your draft into **real payment instructions**: it sets `payTo` to the vault address (PDA) buyers must pay, and adds `extra` (`feePayer`, `programId`, `configAddress`, `merchantWallet`, …).

**Save the JSON you get back.** In Step 3 you return that **same JSON** as the body of `402 Payment Required`. You do **not** need to run `/upgrade` on every visitor—only when something below changes.

**In plain terms**

- **What to save:** The whole JSON object printed by the command above (your upgraded `accepts` block and metadata).
- **Why:** One `/upgrade` per “product” (route, price, mint, network) is enough; doing it on every unpaid request is slower and can hit rate limits.
- **Where to put it:** Anything your app can read—a config file, env var, database row, Redis, or load it when the server starts.
- **When to run `/upgrade` again:** You change facilitator URL (`$BASE`), chain, USDC mint, amount, or the facilitator asks you to refresh—then replace what you saved with the new response.

## Step 3 — Return HTTP 402 to buyers

When a buyer hits your API without payment:

```
HTTP/1.1 402 Payment Required
Content-Type: application/json

<THE_UPGRADED_402_BODY>
```

## Step 4 — Verify payment on retry

When a buyer retries with a `PAYMENT-SIGNATURE` header:

```bash
# Extract the payment proof from the header
PROOF="$(echo $PAYMENT_SIGNATURE_HEADER)"

# Forward to the facilitator
RESULT=$(curl -sS -X POST "$BASE/api/v1/facilitator/settle" \
  -H "Content-Type: application/json" \
  -d "$PROOF")

# Check result
if echo "$RESULT" | jq -e '.success == true' > /dev/null; then
  # Serve the premium resource
  echo "Payment confirmed!"
else
  # Return 402 again
  echo "Payment failed: $(echo $RESULT | jq -r '.errorReason')"
fi
```

## Step 5 — Return settlement proof (optional, x402 v2)

After successful settlement, return the result in a `PAYMENT-RESPONSE` header:

```
HTTP/1.1 200 OK
PAYMENT-RESPONSE: <BASE64_ENCODED_SETTLE_RESULT>
Content-Type: application/json

{"data": "your premium content"}
```

---

## That's it!

**No PDA derivation. No on-chain setup. No Solana SDK.**

The `/upgrade` endpoint handles all institutional routing for you. For sovereign status (lower fees), see the full [onboarding guide](/onboarding_guide.md).

**Full reference:** `GET /seller-quick-start.md` and `GET /openapi.json` on the facilitator.

## Launch checklist

- Publish one facilitator base URL per environment.
- Save the upgraded `402` body and replace it when facilitator capabilities or asset allowlists change.
- Run a preview transaction before Mainnet launch.
- For `sla-escrow`, publish the oracle authority, profile id, evidence registry policy, and maximum supported payment size.
