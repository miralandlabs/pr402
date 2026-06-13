---
title: "Quick reference · 5 steps"
---

# Quick reference · 5 steps

> **Cheat-sheet.** Minimal 5-step flow for sellers who already understand x402. **New seller?** → [Start here · Sellers](/start-here.html) (prerequisites + pick rail). **Full walkthrough?** → [Integrate your API](/seller-quick-start.html) (diagram + language examples).

> **You have a Solana wallet and want to charge for API calls. Here's the fastest path.**

> **Status.** The `exact` rail on pr402 is live on **Solana Mainnet** (`https://ipay.sh`) and **Devnet** (`https://preview.ipay.sh`); the same service is also served at `https://agent.pay402.me` / `https://preview.agent.pay402.me` (not deprecated). For `sla-escrow` you must choose an `oracle_authority` you trust — reference implementations ship in the [`oracles/`](https://github.com/miraland-labs/oracles) workspace (three sibling profiles: api-quality, onchain-transfer, file-delivery).

Replace **`$BASE`** with your facilitator URL. Confirm the cluster with **`GET /api/v1/facilitator/health`** on that host.

Use the same `$BASE` in your public docs, 402 bodies, buyer instructions, `/verify`, and `/settle`. Most integration failures come from mixing preview/mainnet origins or publishing a bare wallet where this facilitator expects a PDA.

**Five steps, in order:** (1) write a small JSON draft → (2) POST it once to `/payment-required/enrich` and **keep the reply** → (3) when someone hasn’t paid, return that reply as **`402`** → (4) when they pay and retry, ask the facilitator to **`/settle`** → (5) optionally echo settlement back to the client.

---

## Step 1 — Draft a minimal JSON body (only for `/payment-required/enrich`, not for buyers)

**What you do:** Copy the template, replace the placeholders (`resource.url`, `payTo`, `asset`, `amount`), and keep this JSON—you will POST it in Step 2.

**This is not what buyers see.** In the abstract x402 spec, some examples show `payTo` as a seller’s wallet address. **pr402’s `exact` (UniversalSettle) rail does not settle to that model:** the facilitator and on-chain program expect buyers to pay into **your SplitVault rail PDAs**. The **`payTo`** you eventually publish must be the **vault PDA** returned from **`GET /api/v1/facilitator/sellers/{wallet}/rails/{scheme}`** or from **`POST /api/v1/facilitator/payment-required/enrich`** — see the [full seller guide](/seller-quick-start.html) (`payTo`: **`<YOUR_VAULT_PDA>`**, with **`extra.merchantWallet`** for your real wallet).

The JSON below is only **input** to **`/payment-required/enrich`**: put your **normal Solana wallet pubkey** in `payTo` here so the facilitator can **replace** it with the correct vault PDA and inject `extra`. **Do not return this draft to buyers** — only the Step 2 response is buyer-facing.

```json
{
  "x402Version": 2,
  "resource": { "url": "https://your-api.com/premium-endpoint" },
  "accepts": [
    {
      "scheme": "exact",
      "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
      "payTo": "<YOUR_WALLET_PUBKEY_ONLY_FOR_UPGRADE_INPUT>",
      "asset": "<USDC_MINT_PUBKEY>",
      "amount": "50000",
      "maxTimeoutSeconds": 300
    }
  ]
}
```

> **Why the placeholder isn’t `<YOUR_VAULT_PDA>` here:** `POST /payment-required/enrich` is what derives/injects the institutional line. Passing your **wallet pubkey** in this draft is how you ask the facilitator to substitute the **canonical vault `payTo`** plus `extra` (`feePayer`, `programId`, `configAddress`, `merchantWallet`, …). What you **serve in HTTP 402** afterward is your **402 payment body**: **`upgrade`’s response JSON** (vault PDA in `payTo`).

## Step 2 — Upgrade it to institutional format (one POST)

**What you do:** Send **Step 1’s JSON** to `/payment-required/enrich`. Store **the JSON body of the HTTP response** (what prints after `curl` / `jq`)—that stored object is what you send to buyers in Step 3.

Save Step 1 as `draft.json` next to this command, or use `-d '{ ... }'` with your JSON inline.

```bash
curl -sS -X POST "$BASE/api/v1/facilitator/payment-required/enrich" \
  -H "Content-Type: application/json" \
  -d @draft.json | jq .
```

The facilitator turns your draft into **real payment instructions**: it sets `payTo` to the vault address (PDA) buyers must pay, and adds `extra` (`feePayer`, `programId`, `configAddress`, `merchantWallet`, …).

**Name for the next steps:** Call the saved response your **402 payment body**. Step 3 uses it **verbatim** (same keys and values as `jq` printed). You do **not** run `/payment-required/enrich` on every visitor—only when something below changes.

**In plain terms**

- **What to save:** The **whole JSON object** returned by `/payment-required/enrich` (root object — not just `accepts`).
- **Why:** One `/payment-required/enrich` per “product” (route, price, mint, network) is enough; doing it on every unpaid request is slower and can hit rate limits.
- **Where to put it:** Anything your app can read—a config file, env var, database row, Redis, or load it when the server starts.
- **When to run `/payment-required/enrich` again:** You change facilitator URL (`$BASE`), chain, USDC mint, amount, or the facilitator asks you to refresh—then replace your **402 payment body** with the new response.

## Step 3 — Return HTTP 402 to buyers

**What you do:** If a request has **no valid payment**, respond with status **`402 Payment Required`**, header **`Content-Type: application/json`**, and body = **your 402 payment body** (the exact JSON from Step 2 — **not** Step 1’s draft).

```
HTTP/1.1 402 Payment Required
Content-Type: application/json

<your 402 payment body — the JSON object returned by POST .../payment-required/enrich in Step 2>
```

## Step 4 — Verify payment on retry

**What you do:** When the buyer retries with a **`PAYMENT-SIGNATURE`** header (their wallet or agent sends payment proof), **`POST`** that proof JSON to **`$BASE/api/v1/facilitator/settle`**. If the response has **`success: true`**, serve your premium response; if not, answer with **`402`** again and the **same 402 payment body** as in Step 3.

```bash
# Example only — replace with however your framework reads the header value.
PROOF='<paste JSON proof from PAYMENT-SIGNATURE as your stack exposes it>'

RESULT=$(curl -sS -X POST "$BASE/api/v1/facilitator/settle" \
  -H "Content-Type: application/json" \
  -d "$PROOF")

# Check result
if echo "$RESULT" | jq -e '.success == true' > /dev/null; then
  # Serve the premium resource
  echo "Payment confirmed!"
else
  # Return 402 again
  echo "Payment failed: $(echo "$RESULT" | jq -r '.errorReason')"
fi
```

## Step 5 — Return settlement proof (optional, x402 v2)

**What you do:** After **`/settle`** succeeds, some clients expect a **`PAYMENT-RESPONSE`** header on your **`200 OK`**. Value is typically the settle result **base64-encoded** (exact shape depends on your buyer stack—see x402 v2 and `{FACILITATOR}/openapi.json`).

```
HTTP/1.1 200 OK
PAYMENT-RESPONSE: <base64 encoding of the settle result from Step 4>
Content-Type: application/json

{"data": "your premium content"}
```

---

## That's it!

**No PDA derivation. No on-chain setup. No Solana SDK.**

The `/payment-required/enrich` endpoint handles all institutional routing for you. For sovereign status (lower fees), see the full [onboarding guide](/onboarding_guide).

**Human-readable docs:** [Hands-on seller lab](/seller-lab.html) · [Integrate your API](/seller-quick-start.html) · [Start here · Sellers](/start-here.html) · [API overview](/api-reference).

**Machine-readable:** `{FACILITATOR}/openapi.json` — substitute your facilitator `$BASE` (for example `https://ipay.sh` or `https://preview.ipay.sh`).

## Launch checklist

- Publish one facilitator base URL per environment.
- Save your **402 payment body** and replace it when facilitator capabilities or asset allowlists change.
- Run a preview transaction before Mainnet launch.
- For `sla-escrow`, publish the oracle authority, profile id, evidence registry policy, and maximum supported payment size.
