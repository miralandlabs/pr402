# pr402 `payTo` — Zoom-in brief (diagrams & seller messaging)

High-level **Web2 vs x402** infographics are useful, but they hide pr402’s most important differentiator: **`payTo` in the HTTP 402 `PaymentRequired` response is not “send USDC to the seller’s wallet.”** On pr402 (Solana), it is an **on-chain routing instruction** to a **program PDA** that enforces splits, protocol fees, escrow, and sustainable economics.

This doc is for:

- **Infographic / blog authors** zooming into “Step 2 — Request with Payment”
- **Sellers** moving from Stripe / bare-wallet x402 examples to pr402

**Canonical machine-readable spec:** `GET {BASE}/api/v1/facilitator/agent-payTo-semantics.json`

---

## Standard x402 vs pr402 (mental model)

### Generic x402 (CDP-style mental model)

```
Buyer ──► HTTP 402 ──► accepts[].payTo ≈ seller wallet (or facilitator-managed path)
         ──► sign & pay ──► facilitator verify/settle
         ──► gas often subsidized by a big operator
```

**Problem:** subsidized gas + “pay the wallet” does not scale. Someone must fund every transaction forever, and bare-wallet `payTo` does not encode protocol fee, escrow, or subscription windows.

### pr402 (Solana) — `payTo` is a program destination

When your API returns **HTTP 402**, the buyer/agent reads `accepts[]`. On pr402, **`payTo` is always an on-chain account owned by a pr402 program** — never “just paste your wallet and hope.”

```mermaid
flowchart TB
  subgraph step2 [Step 2 Zoom-In: pr402 PaymentRequired.accepts]
    A[Buyer hits protected route] --> B[HTTP 402 + accepts array]
    B --> C{scheme?}
    C -->|exact| D["payTo = SplitVault PDA"]
    C -->|sla-escrow| E["payTo = Escrow PDA per mint"]
    D --> F["extra.merchantWallet = your real wallet"]
    E --> G["extra.merchantWallet or beneficiary = payout wallet"]
    E --> H["Funds held until delivery + oracle verdict"]
    D --> I["UniversalSettle splits: seller + protocol fee on-chain"]
  end
```

| Field | Standard x402 (typical) | pr402 `exact` | pr402 `sla-escrow` |
|-------|-------------------------|---------------|---------------------|
| **`payTo`** | Often seller wallet / facilitator abstraction | **SplitVault PDA** (UniversalSettle) | **Escrow PDA** (per payment mint) |
| **Seller wallet** | Same as `payTo` in naive examples | In **`extra.merchantWallet`** only | In **`extra.merchantWallet`** / **`beneficiary`** — **not** `payTo` |
| **Protocol fee** | Opaque / subsidized elsewhere | **On-chain split** (90 bps sovereign / 100 bps JIT) | Escrow + oracle economics |
| **Gas / tx fees** | Often **subsidized** by operator | **Default: buyer pays** (`facilitatorPaysTransactionFees: false`) — sustainable BYOG | Same default; optional sponsorship gated |
| **Buyer protection** | Pay → hope seller delivers | Instant settle after verify | **Funds locked** until oracle confirms delivery |
| **Subscriptions** | N/A in generic diagram | One **`exact`** payment to vault PDA → seller issues **time-window JWT** | Rare; per-call / per-delivery use escrow |

---

## Zoom-in: Step 2 — “402 Payment Required”

### End-to-end flow (pr402)

```
Buyer/Agent                    Your API                      pr402 Facilitator
     |                            |                                  |
     |--- GET /api/premium ----->|                                  |
     |<-- 402 + accepts[] -------|  payTo = Vault/Escrow PDA        |
     |                            |  extra.merchantWallet = seller   |
     |--- build/sign tx ------------------------------------------->|
     |<-- unsigned tx + verifyBodyTemplate (optional) --------------|
     |                            |                                  |
     |--- GET /api/premium ----->|                                  |
     |    PAYMENT-SIGNATURE: …    |--- POST /settle --------------->|
     |                            |    (verify + on-chain execute)   |
     |                            |<-- 200 settled ------------------|
     |<-- 200 + resource ---------|                                  |
```

### Profound meaning: payment “diversion”

| Model | Where money goes first | Access |
|-------|------------------------|--------|
| **Traditional Web2** | Stripe / merchant account | API key after billing |
| **Naive x402** | Seller wallet (often subsidized gas) | Verify/settle → access |
| **pr402** | **Program vault PDA** (`payTo`) | On-chain enforce split/escrow → access |

**pr402 `payTo` diverts funds into a smart-contract vault** that:

1. **Routes payout** — seller share + protocol fee atomically on-chain (UniversalSettle on `exact`).
2. **Separates identity from destination** — your wallet is **`extra.merchantWallet`** (who earns); **`payTo`** is **where the program accepts funds** (how it’s enforced).
3. **Enables `sla-escrow`** — `payTo` = escrow PDA; release/refund is program + oracle logic, not trust.
4. **Powers x402 subscriptions** — one payment to the **same vault `payTo`** on `/subscribe`; data routes use **JWT**, not per-request 402. See hub [`SUBSCRIPTION_PATTERN.md`](../../SUBSCRIPTION_PATTERN.md) (x402 workspace).
5. **Stays sustainable** — pr402 does **not** assume a large operator pays Solana gas forever; buyers bring their own fees unless explicitly sponsored.

### Why subsidized gas is unsustainable (diagram caption)

Standard x402 demos often show a facilitator paying every buyer’s network fee. That is a **loss-leader**, not a protocol invariant. pr402’s default is **BYOG (bring your own gas)**: the buyer signs and pays Solana fees; the facilitator verifies and settles. Optional fee sponsorship exists on some builds but is **gated** — not “free forever.”

### Rail-specific `payTo` (from `agent-payTo-semantics.json`)

**`exact` (UniversalSettle / SplitVault)**

- **kind:** `splitVault`
- **resolve:** `GET /api/v1/facilitator/sellers/{wallet}/rails/exact` → `vaultPda`
- **verify:** destination must match UniversalSettle split vault
- **use:** pay-per-call APIs, subscription purchase endpoints (`POST /subscribe?tier=…`)

**`sla-escrow`**

- **kind:** `escrowPda`
- **resolve:** `GET /api/v1/facilitator/sellers/{wallet}/rails/sla-escrow?asset=<mint>` → escrow PDA for that mint
- **payout wallet:** `extra.merchantWallet` or `extra.beneficiary` — **not** `payTo`
- **verify:** `payTo` must equal `derive_escrow_pda(mint, bank)`
- **use:** token delivery, high-value jobs, SLA-backed fulfillment (e.g. x402-buy-spl-token)

---

## Seller workflow (do not hand-craft PDAs)

1. Draft 402 with **your wallet pubkey** in `payTo` (**upgrade input only** — not buyer-facing).
2. `POST /api/v1/facilitator/payment-required/enrich` → facilitator returns **canonical vault/escrow PDA** + institutional `extra`.
3. Store that JSON; serve it on every unpaid request.

For `exact`, you can also resolve via:

```bash
curl -sS "$BASE/api/v1/facilitator/sellers/YOUR_PUBKEY/rails/exact" | jq .
```

**Buyer-facing 402 body example (`exact`):**

```json
{
  "x402Version": 2,
  "accepts": [{
    "scheme": "exact",
    "network": "solana:…",
    "asset": "<USDC_MINT>",
    "amount": "50000",
    "payTo": "<SPLIT_VAULT_PDA>",
    "maxTimeoutSeconds": 300,
    "extra": {
      "feePayer": "…",
      "programId": "…",
      "configAddress": "…",
      "feeBps": "100",
      "merchantWallet": "<YOUR_ACTUAL_WALLET>"
    }
  }]
}
```

> **Rule:** Never publish bare wallet as final `payTo` on pr402 `exact` rail. Buyers pay the **PDA**; the program + facilitator verify that destination.

**Further reading**

- [Seller quick start](../docs-site/seller-quick-start.md) — Step 1 (`payTo`) and Step 2 (`/payment-required/enrich`)
- [Start here · SplitVault model](../docs-site/start-here.md)
- [Choosing x402 on Solana](../docs-site/pr402-vs-alternatives.md) — fee / gas model vs CDP

---

## One-line takeaway for sellers

> **`payTo` on pr402 is where the buyer’s money enters the protocol** (vault or escrow PDA). **Your wallet is how you get paid** (`extra.merchantWallet`), not where buyers send funds. That diversion — from “merchant account / bare wallet” to **program-enforced routing** — is what makes pr402 sustainable, escrow-capable, and subscription-ready on Solana.
