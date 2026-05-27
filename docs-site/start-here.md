---
title: "Start here · Seller checklist"
---

# Start here · Seller checklist

**Audience:** You run a normal web API (REST, GraphQL, etc.) in **any language** and want paid access without becoming a blockchain developer.

**Goal:** Integrate against **Mainnet** (`https://ipay.sh`) and accept real payments. **Devnet** (`https://preview.ipay.sh`) is an optional rehearsal if you want zero financial risk while learning the steps.

> You keep your API. You add HTTP 402 + forward payment proof to pr402. You never run any blockchain inside your server.

---

## Why pr402? · True differentiators

Other x402 facilitators exist (notably [Coinbase CDP](https://docs.cdp.coinbase.com/x402/network-support) and [x402.org](https://x402.org/facilitator)). Solana Foundation’s [`pay` CLI](https://pay.sh) is a **buyer client**, not a facilitator like [`ipay.sh`](https://ipay.sh) — see [Choosing x402 on Solana](/pr402-vs-alternatives.html). Summary of pr402-specific strengths below.

### Preview rehearsal ([`preview.ipay.sh`](https://preview.ipay.sh))

| | **pr402** | **CDP / x402.org** (facilitators) | **`pay` CLI** (buyer client) |
|---|---|---|---|
| Solana Devnet for x402 | Yes | Yes | Demos / local tooling only |
| Paired prod ↔ preview hostnames | **`ipay.sh` ↔ `preview.ipay.sh`** | CDP: one URL + `network`; x402.org: testnet URL | N/A — not a facilitator |
| Signup required for public Devnet | **No** (just access [preview.ipay.sh](https://preview.ipay.sh)) | CDP: API keys; x402.org: no signup | N/A |
| **`sla-escrow` on Devnet** | **Yes** | **`exact` only** on Solana | N/A |
| Seller lifecycle UI on Devnet | **Yes** | No | No |

Preview rehearsal is a **full production mirror** (including both rails, the seller UI and one-URL migration), so you can test it without risking your real SOL or USDC on Solana Mainnet.
---

### For sellers (resource providers)

| Differentiator | What it means |
|---|---|
| **No chain code in your API** | Return 402, forward `PAYMENT-SIGNATURE` to `/settle`. pr402 verifies and executes on-chain. |
| **`/upgrade` + discovery** | No hand-built `extra` or vault PDA math — facilitator injects institutional `accepts[]`. |
| **Two settlement rails** | **`exact`** (UniversalSettle, instant) and **`sla-escrow`** (on-chain escrow + oracle). CDP/x402.org Solana today is **`exact` only**. |
| **Sovereign fee tier** | Self-provision via **Activate** qualifies **90 bps** protocol fee tier vs **100 bps** JIT standard on `exact`. |
| **Blockhash-safe `/settle`** | Verification runs inside `/settle` before broadcast — avoids verify→deliver→settle gaps that break on Solana (~60s blockhash). |
| **SplitVault `payTo` model** | Buyers pay program PDAs, not your bare wallet — enforced by UniversalSettle + facilitator verify. |
| **Honest fee floors** | $0.01 min on `exact`, $0.10 on `sla-escrow` protocol fee — we do not subsidize sub-cent micro-payments like some large facilitators. |

**Seller rehearsal path:** integrate on **[`https://preview.ipay.sh`](https://preview.ipay.sh)**, then switch to **[`https://ipay.sh`](https://ipay.sh)** and re-run `/upgrade` for Mainnet mints/PDAs.

---

### For buyers (payer agents)

| Differentiator | What it means |
|---|---|
| **`sla-escrow` buyer protection** | Funds stay in on-chain escrow until delivery terms are met or an oracle verdict — refund/release paths are program-enforced. **No standard x402 facilitator offers an equivalent escrow rail today.** |
| **Tx builders on the facilitator** | `POST /build-exact-payment-tx` and `POST /build-sla-escrow-payment-tx` return unsigned txs + `verifyBodyTemplate` — no guessing instruction layout. |
| **Open buyer tooling** | [`@pr402/client`](https://www.npmjs.com/package/@pr402/client), `pr402-client` (Rust), [`x402-buyer-starter`](https://github.com/miraland-labs/x402-buyer-starter). |
| **Choose your oracle** | On `sla-escrow`, buyers pick `oracle_authority` from seller-advertised profiles or `GET /capabilities → slaEscrowOracleProfiles[]`. |
| **Zero-risk rehearsal** | Point builders/settle at `preview.ipay.sh` with Devnet USDC — same flow as production. |

[`pay` CLI](https://pay.sh) complements pr402 on the **buyer side** — it can call APIs that settle via **`ipay.sh`**. [Choosing x402 on Solana](/pr402-vs-alternatives.html).

---

### For oracles (operators & developers)

| Differentiator | What it means |
|---|---|
| **First x402 escrow oracle economy** | The on-chain **`sla-escrow`** program is hash-agnostic; oracles implement off-chain evaluation and submit `ConfirmOracle` on-chain. pr402 is the facilitator wired to this rail. |
| **Open-source reference workspace** | [`miraland-labs/oracles`](https://github.com/miraland-labs/oracles) — shared `oracle-common` library plus three sibling binaries: **api-quality**, **onchain-transfer**, **file-delivery**. Clone the closest profile, swap evaluation logic, register a new `profileId`. |
| **Profile discovery via facilitator** | `GET /capabilities → slaEscrowOracleProfiles[]` advertises canonical profile ids, operator pubkeys, and normative spec URLs. |
| **Verdict-neutral oracle tips** | Oracle tip is paid when a verdict is rendered — adjudication work, not outcome (release vs refund). |
| **Built-in operator option** | pr402 deployments may operate **`x402/oracles/onchain-transfer/v1`** as a default for SPL transfer delivery; other profiles are ecosystem-operated. Listing via [register-oracle](https://github.com/miralandlabs/pr402/issues/new?template=register-oracle.md) is configuration consistency, not reliability endorsement. |
| **Full pipeline in the template** | Chain monitor → hash-bound SLA/delivery registry → evaluate → settle — documented in the oracles repo ([Seller guide](https://github.com/miraland-labs/oracles/blob/main/docs/SELLER_GUIDE.md), [Buyer guide](https://github.com/miraland-labs/oracles/blob/main/docs/BUYER_GUIDE.md)). |

**Oracle rehearsal:** run your binary against **Devnet** while sellers/buyers use **`preview.ipay.sh`** — same profile ids and escrow program logic as Mainnet.

**Full comparison guide:** [Choosing x402 on Solana](/pr402-vs-alternatives.html) (facilitators vs buyer tools — `pay` CLI is not a facilitator peer)

---

## Documentation map

| When you want… | Read |
|---|---|
| **This checklist** (prerequisites → fees → pick rail → numbered steps) | **This page** |
| Full 30-minute walkthrough + Rust / Python / JS / Go samples | [Seller Quickstart](/seller-quick-start.html) |
| Minimal cheat-sheet (`exact` rail only) | [Seller shortcut · 5 steps](/quickstart-seller.html) |
| Sovereign fees, JIT provisioning, oracle profiles | [Onboarding guide](/onboarding_guide.html) |
| **Choosing x402 on Solana** (facilitators vs buyer tools) | [Comparison guide](/pr402-vs-alternatives.html) |
| Machine-readable contract | `GET /openapi.json` on your facilitator host |

**Facilitator hosts:** Mainnet **`https://ipay.sh`** · Devnet **`https://preview.ipay.sh`**. Use one origin everywhere (`$BASE`). Confirm cluster with `GET $BASE/api/v1/facilitator/health`.

---

## Step 0 — Prerequisites (before code)

Check these off:

- [ ] **An HTTP API** you control (Node, Python, Go, Rust, Java, Ruby, PHP, …).
- [ ] **A Solana wallet** (your seller identity):
  - **Browser (typical):** [Phantom](https://phantom.app/) or [Solflare](https://solflare.com/) — install the extension, create or import a wallet, copy your **public address**.
  - **Server / CI only:** install [Solana CLI](https://docs.solana.com/cli/install-solana-cli-tools), then run `solana-keygen new -o seller.json` — store the keypair safely; publish only the pubkey (`solana-keygen pubkey seller.json`).
- [ ] **Mainnet SOL in that wallet (recommended, not required):** ~**0.1 SOL** is enough if you use **Activate** on **[ipay.sh](https://ipay.sh)** to self-provision your SplitVault before the first sale. That unlocks the **sovereign** protocol fee tier (**90 bps** vs **100 bps** standard). If you skip Activate, pr402 can still **JIT-provision** your vault on the first paid transaction — you keep 100 bps fees and pay no upfront SOL.
- [ ] **Do not hardcode mints or PDAs from blog posts** — copy live values from `GET $BASE/api/v1/facilitator/capabilities` or `/supported`.

You do **not** need a blockchain SDK in your API server.

**Optional — rehearse on Devnet first:** If you are unsure of the steps, point `$BASE` at `https://preview.ipay.sh` and use **[preview.ipay.sh](https://preview.ipay.sh)** (not Mainnet ipay.sh) for Preview / Activate. Devnet SOL is free test money:

- **Browser wallet:** switch the wallet to **Devnet**, then use [faucet.solana.com](https://faucet.solana.com/) (GitHub sign-in) or your wallet’s built-in Devnet airdrop if offered.
- **Solana CLI:** `solana config set --url devnet`, then `solana airdrop 2 YOUR_PUBKEY`.

Only Devnet Activate needs this SOL; your API server never spends it.

---

## Protocol fees & how to price your API

pr402 deducts a **protocol fee** from each payment at settlement. Treat **`GET $BASE/api/v1/facilitator/capabilities`** as authoritative if numbers drift; the table below matches the live **ipay.sh** deployment today.

| | **`exact`** | **`sla-escrow`** |
|---|---|---|
| **Standard rate** | **100 bps** (1.00%) | **100 bps** (1.00%) on protocol fee |
| **Sovereign rate** (`exact` only) | **90 bps** (0.90%) after self-provision via **Activate** | — |
| **Minimum protocol fee (USDC rail)** | **$0.01** (1 cent) | **$0.10** (10 cents) |
| **Oracle tip** | none | **100 bps** (1.00%) when an oracle renders a verdict — no floor |

**How the floor bites on small `exact` payments:** fee = max(1% × amount, **$0.01**). Examples on USDC:

| Price per call | Protocol fee | Fee as % of your revenue |
|---|---|---|
| $0.01 | $0.01 | 100% |
| $0.02 | $0.01 | 50% |
| $0.05 | $0.01 | 20% |
| $0.10+ | scales with 1% | ≤ 10% and falling |

**Rough pricing guidance (draft operations guide):**

- **`exact`:** aim for **≥ ~$0.05 USDC per call** so the $0.01 floor is not more than ~20% of revenue. Below **~$0.02**, more than half of each payment can go to protocol fees. pr402 is **not** optimized for sub-cent micro-payments — unlike some large facilitators we do not fully subsidize tx gas, and we enforce a **1 cent** protocol floor to cover running costs.
- **`sla-escrow`:** aim for **≥ ~$10 USDC per payment** (escrow + oracle economics). For smaller tickets, **`exact`** is usually the better rail.

**Why offer `sla-escrow`?** Buyers get **on-chain escrow protection** — funds are not released until delivery terms are met or an oracle rules. Standard x402 facilitators today only offer instant-settle rails like `exact`; **pr402 is the only facilitator shipping this escrow model**, which matters for high-value or slow-fulfillment services where buyers need refund/release guarantees.

These are recommendations, not hard limits. You choose price and rail; just understand the fee math before you launch.

---

## Step 1 — Pick your rail

| | **`exact`** (UniversalSettle) | **`sla-escrow`** (SLA-Escrow) |
|---|---|---|
| **Best for** | Instant access, API calls, payments from ~5¢ upward | High value, slow fulfillment, refunds / delivery proofs |
| **Buyer experience** | Pay once → content immediately | Pay into escrow → you deliver → oracle / release path |
| **Buyer protection** | Standard instant x402 settle | **Escrow on-chain** — pr402-only among x402 facilitators today |
| **Your integration size** | Smaller (402 + settle) | Larger (SLA terms, oracle, fulfillment) |
| **Follow** | [Seller shortcut · 5 steps](/quickstart-seller.html) after this checklist | [Onboarding guide · SLA-Escrow](/onboarding_guide.html#sla-escrow-oracle-profile-and-default-operator-hints) + [Seller Quickstart](/seller-quick-start.html) |

**Default for first integration:** `exact`. Switch to `sla-escrow` when buyers need escrow protection on high-value or slow-fulfillment work — a selling point no other standard x402 facilitator provides yet.

---

## Step 2 — Get your `payTo` on ipay.sh

Your buyers must pay into a **program PDA** (`payTo`), not your bare wallet.

1. Open **[ipay.sh](https://ipay.sh)** (Mainnet — production) or **[preview.ipay.sh](https://preview.ipay.sh)** (Devnet — rehearsal only).
2. Scroll to **§ seller lifecycle** (or `https://ipay.sh#seller-lifecycle`).
3. Paste your **seller pubkey** (or connect wallet).
4. Run **Preview** — note the vault / `payTo` the page shows (no on-chain change).

For `exact`, you can also resolve via:

```bash
export BASE="https://ipay.sh"   # Mainnet

curl -sS "$BASE/api/v1/facilitator/discovery?wallet=YOUR_PUBKEY&scheme=exact" | jq .
```

**Recommended for Mainnet `exact`:** run **Activate** on the same site (Step 2 of seller lifecycle). Your wallet signs one provisioning transaction (~0.1 SOL for rent + fees). That makes you **sovereign** (**90 bps** protocol fee on every later payment). Skipping Activate is fine — pr402 JIT-provisions on first settle at **100 bps**.

---

## Step 3 — Build your 402 payment body (once per product)

Do **not** hand-craft `extra` fields. POST a minimal draft to **`/upgrade`** once and **save the response**.

```bash
export BASE="https://ipay.sh"   # Mainnet

curl -sS -X POST "$BASE/api/v1/facilitator/upgrade" \
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

## Step 5 — Test and go live on Mainnet

```bash
# Unpaid — expect 402
curl -sS -D - "https://your-api.com/premium-endpoint" -o /dev/null

# Paid path — use x402-seller-starter, a buyer agent, or @pr402/client
# See Buyer Quickstart if you need a payer
```

Run one real (small) Mainnet payment end-to-end before announcing the product.

**Optional Devnet rehearsal:** If you practiced on `preview.ipay.sh`, switch `$BASE` to `https://ipay.sh`, re-run **`/upgrade`** on Mainnet (mints and PDAs differ), update your stored 402 body, and test once more.

---

## When you are stuck

| Symptom | Likely fix |
|---|---|
| Buyers pay wrong address | `payTo` must be vault PDA from `/upgrade` or `/discovery`, not bare wallet |
| Mixed Devnet / Mainnet | One `$BASE` everywhere — 402 body, settle, health check |
| Settle fails quickly on Solana | Call `/settle` promptly; do not verify-then-wait-then-settle with long gaps |
| Fee eats most of a micro-payment | Raise price or accept the $0.01 floor math; see **Protocol fees** above |
| Need audit / correlation IDs | Optional `POST /verify` before `/settle` — see [Seller Quickstart](/seller-quick-start.html) |

Deep reference: [Agent integration runbook](/agent-integration.html) · [API overview](/api-reference.html)
