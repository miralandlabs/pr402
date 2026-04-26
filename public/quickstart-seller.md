# Seller Quick Start — Monetize Your API with x402

> **You have a Solana wallet and want to charge for API calls. Here's the fastest path.**

> **Launch phase:** **Experimental** — **use at your own risk**.

Replace **`$BASE`** with your facilitator URL. Official deployments: **Production** `https://agent.pay402.me` (Mainnet) · **Preview** `https://preview.agent.pay402.me` (Devnet). Confirm **`solanaNetwork`** with **`GET /api/v1/facilitator/health`** on that host.

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

The facilitator replaces your bare wallet `payTo` with the correct vault PDA and injects all `extra` metadata (`feePayer`, `programId`, `configAddress`, `merchantWallet`). **Cache this response.**

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
