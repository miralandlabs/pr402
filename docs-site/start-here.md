---
title: "Start here · Seller checklist"
---

# Start here · Seller checklist

**Audience:** You run a normal web API (REST, GraphQL, etc.) in **any language** and want paid access without becoming a blockchain developer.

**Time:** One afternoon on **Devnet** first. Then flip one URL for Mainnet.

> You keep your API. You add HTTP 402 + forward payment proof to pr402. You never run any blockchain inside your server.

---

## Documentation map

| When you want… | Read |
|---|---|
| **This checklist** (prerequisites → pick rail → numbered steps) | **This page** |
| Full 30-minute walkthrough + Rust / Python / JS / Go samples | [Seller Quickstart](/seller-quick-start.html) |
| Minimal cheat-sheet (`exact` rail only) | [Seller shortcut · 5 steps](/quickstart-seller.html) |
| Sovereign fees, JIT provisioning, oracle profiles | [Onboarding guide](/onboarding_guide.html) |
| Machine-readable contract | `GET /openapi.json` on your facilitator host |

**Facilitator hosts:** Devnet **`https://preview.ipay.sh`** · Mainnet **`https://ipay.sh`**. Use one origin everywhere (`$BASE`). Confirm cluster with `GET $BASE/api/v1/facilitator/health`.

---

## Step 0 — Prerequisites (before code)

Check these off:

- [ ] **An HTTP API** you control (Node, Python, Go, Rust, Java, Ruby, PHP, …).
- [ ] **A Solana wallet** (your seller identity):
  - **Browser:** [Phantom](https://phantom.app/) or [Solflare](https://solflare.com/) extension — copy your public address.
  - **Server-only:** `solana-keygen new` — store the keypair safely; publish only the pubkey.
- [ ] **Devnet first:** integrate against `https://preview.ipay.sh`, not Mainnet.
- [ ] **Optional Devnet SOL** (only if you use on-site **Activate** on ipay.sh): faucet via your wallet or `solana airdrop`.
- [ ] **Do not hardcode mints or PDAs from blog posts** — copy live values from `GET $BASE/api/v1/facilitator/capabilities` or `/supported`.

You do **not** need a blockchain SDK in your API server.

---

## Step 1 — Pick your rail

| | **`exact`** (UniversalSettle) | **`sla-escrow`** (SLA-Escrow) |
|---|---|---|
| **Best for** | Instant access, micro-payments, API calls | High value, slow fulfillment, refunds / delivery proofs |
| **Buyer experience** | Pay once → content immediately | Pay into escrow → you deliver → oracle / release path |
| **Your integration size** | Smaller (402 + settle) | Larger (SLA terms, oracle, fulfillment) |
| **Follow** | [Seller shortcut · 5 steps](/quickstart-seller.html) after this checklist | [Onboarding guide · SLA-Escrow](/onboarding_guide.html#sla-escrow-oracle-profile-and-default-operator-hints) + [Seller Quickstart](/seller-quick-start.html) |

**Default for first integration:** `exact`. Switch to `sla-escrow` only when you need time-delayed delivery or escrow semantics.

---

## Step 2 — Get your `payTo` on ipay.sh

Your buyers must pay into a **program PDA** (`payTo`), not your bare wallet.

1. Open **[ipay.sh](https://ipay.sh)** (Mainnet) or **[preview.ipay.sh](https://preview.ipay.sh)** (Devnet).
2. Scroll to **§ seller lifecycle**.
3. Paste your **seller pubkey** (or connect wallet).
4. Run **Preview** — note the vault / `payTo` the page shows (no on-chain change).

For `exact`, you can also resolve via:

```bash
curl -sS "$BASE/api/v1/facilitator/discovery?wallet=YOUR_PUBKEY&scheme=exact" | jq .
```

**Optional (recommended for `exact`):** **Activate** on ipay.sh provisions your SplitVault on-chain and unlocks the sovereign fee discount. Not required to start testing 402 + settle on Devnet.

---

## Step 3 — Build your 402 payment body (once per product)

Do **not** hand-craft `extra` fields. POST a minimal draft to **`/upgrade`** once and **save the response**.

```bash
export BASE="https://preview.ipay.sh"   # Devnet

curl -sS -X POST "$BASE/api/v1/facilitator/upgrade" \
  -H "Content-Type: application/json" \
  -d '{
    "x402Version": 2,
    "resource": { "url": "https://your-api.com/premium-endpoint" },
    "accepts": [{
      "scheme": "exact",
      "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
      "payTo": "YOUR_WALLET_PUBKEY",
      "asset": "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
      "amount": "50000",
      "maxTimeoutSeconds": 300
    }]
  }' | jq . > payment-body.json
```

Store `payment-body.json` as your **402 payment body**. Re-run `/upgrade` only when price, mint, network, or facilitator URL changes.

Details: [Seller shortcut · Steps 1–2](/quickstart-seller.html).

---

## Step 4 — Change your API (three edits, any language)

1. **No payment proof** → return **HTTP 402** with body = your 402 payment body (from Step 3).
2. **Request includes `PAYMENT-SIGNATURE`** → POST that JSON to **`$BASE/api/v1/facilitator/settle`**.
3. **Settle returns 200** → return your premium content (optionally echo `PAYMENT-RESPONSE` header).

Pseudocode:

```
if request has no PAYMENT-SIGNATURE header:
    return 402 with payment-body.json

result = POST $BASE/api/v1/facilitator/settle with PAYMENT-SIGNATURE body
if result is not 200:
    return 402 again

return 200 with your protected content
```

Language examples: [Seller Quickstart · language section](/seller-quick-start.html#language-examples). Java, Ruby, and others: same three edits — only syntax differs.

---

## Step 5 — Test on Devnet

```bash
# Unpaid — expect 402
curl -sS -D - "https://your-api.com/premium-endpoint" -o /dev/null

# Paid path — use x402-seller-starter, a buyer agent, or @pr402/client
# See Buyer Quickstart if you need a payer
```

Fix Devnet end-to-end before changing `$BASE` to `https://ipay.sh`.

---

## Step 6 — Go Mainnet

- [ ] Switch `$BASE` to **`https://ipay.sh`**
- [ ] Re-run **`/upgrade`** on Mainnet (mints and PDAs differ)
- [ ] Update your stored 402 payment body
- [ ] Re-test unpaid + paid once

---

## When you are stuck

| Symptom | Likely fix |
|---|---|
| Buyers pay wrong address | `payTo` must be vault PDA from `/upgrade` or `/discovery`, not bare wallet |
| Mixed Devnet / Mainnet | One `$BASE` everywhere — 402 body, settle, docs |
| Settle fails quickly on Solana | Call `/settle` promptly; do not verify-then-wait-then-settle with long gaps |
| Need audit / correlation IDs | Optional `POST /verify` before `/settle` — see [Seller Quickstart](/seller-quick-start.html) |

Deep reference: [Agent integration runbook](/agent-integration.html) · [API overview](/api-reference.html)
