---
title: "Choosing x402 on Solana"
---

# Choosing x402 on Solana

**Audience:** Sellers, buyers, and oracle operators evaluating how to integrate x402 on Solana.

x402 involves **two different layers**. Comparing them in one “facilitator” row is misleading — especially for **`pay.sh`**, which is **not** a settlement host like **`ipay.sh`**.

| Layer | Question it answers | Examples |
|---|---|---|
| **Facilitator** | Where does **verify / settle** run? (Seller forwards payment proof here.) | **pr402** (`ipay.sh`), **Coinbase CDP**, **x402.org**, self-hosted |
| **Buyer client** | How does the **payer** sign and retry after 402? | **`pay` CLI** (`pay.sh` / `@solana/pay`), **`@pr402/client`**, raw curl |

This page compares **facilitators fairly** (pr402 · CDP · x402.org), then covers **buyer tooling** separately — including how **`pay` CLI** works **with** pr402, not **instead of** it.

> **Do not confuse hostnames:** **`ipay.sh`** = pr402 facilitator · **`pay.sh`** = Solana Foundation buyer CLI + API catalog.

---

## Terminology

| Name | Layer | What it is |
|---|---|---|
| **pr402** | Facilitator | Hosted Solana settlement — `https://ipay.sh` (Mainnet) · `https://preview.ipay.sh` (Devnet). **`exact` + `sla-escrow`**, tx builders, seller onboarding. |
| **Coinbase CDP x402** | Facilitator | Hosted **multi-chain** facilitator — strong on Base/EVM; Solana **`exact` only**. |
| **x402.org facilitator** | Facilitator | Community **testnet-only** host — signup-free Devnet / **`exact`** rehearsal. |
| **`pay` CLI** (`pay.sh`) | Buyer client | Solana Foundation tool — wraps `curl` / agents, local wallet signing, MCP, debugger. **Does not host `/verify` or `/settle`.** |

---

## Part A — Facilitator comparison (sellers pick here)

**Use this table when you ask:** “Where should my API forward `PAYMENT-SIGNATURE` for verify and settle?”

| | **pr402** | **Coinbase CDP** | **x402.org** |
|---|---|---|---|
| **Role** | Solana-native facilitator + ecosystem | Multi-chain facilitator | Testnet facilitator |
| **Production Solana Mainnet** | Yes — `https://ipay.sh` | Yes — same CDP API URL | No (testnet only) |
| **Solana Devnet** | Yes — `https://preview.ipay.sh` | Yes — network id in `accepts[]` | Yes — `https://x402.org/facilitator` |
| **Paired prod ↔ preview hostnames** | **Yes** — flip `$BASE` | One URL; change `network` | Testnet-only URL |
| **Public Devnet without signup** | **Yes** (`preview.ipay.sh`) | No (CDP API keys) | **Yes** |
| **Solana `exact` rail** | Yes (UniversalSettle / SplitVault) | Yes | Yes |
| **Solana `sla-escrow` rail** | **Yes** | No | No |
| **On-chain buyer escrow** | **Yes** | No on Solana | No |
| **Open oracle reference stack** | **Yes** ([`oracles/`](https://github.com/miraland-labs/oracles)) | No | No |
| **Seller `/upgrade` (no PDA math)** | **Yes** | Different integration shape | Limited |
| **Seller lifecycle UI** | **Yes** (Preview / Activate / Verify on ipay.sh) | No | No |
| **Multi-chain (Base, Polygon, …)** | Solana-focused | **Yes** | Base Sepolia + Solana Devnet |
| **Compliance / KYT** | Not positioned as core | **Yes** (CDP) | No |
| **Fee / gas model** | Transparent protocol floors; BYOG | CDP free tier + per-tx pricing; subsidized paths on some networks | Testnet |

### When to choose which facilitator

**Choose pr402** when you need **Solana depth**:

1. **`sla-escrow`** — buyer escrow until delivery or oracle verdict (not available on CDP/x402.org Solana today).
2. **Simplest seller server** — 402 + forward proof; **`/upgrade`**; blockhash-safe **`/settle`**.
3. **Production-mirror rehearsal** — **`preview.ipay.sh`** matches **`ipay.sh`** (both rails, same OpenAPI, seller UI).
4. **Oracle economy** — open-source profiles + `slaEscrowOracleProfiles[]` on `/capabilities`.
5. **SplitVault economics** — sovereign **90 bps** after Activate, JIT alternative, clear fee floors.

**Choose CDP** when you need **EVM-first multi-chain** x402, enterprise compliance, or you already use Coinbase Developer Platform — and Solana **`exact`** instant pay is enough.

**Choose x402.org** for a **signup-free Devnet spike** on **`exact` only** — not Mainnet production, not escrow. Migrate to pr402 or CDP when you go live.

---

## Part B — Buyer client comparison (payers pick here)

**Use this table when you ask:** “How does my agent or CLI pay a 402?”

These tools sit **in front of** whatever facilitator the **seller** configured. They are **not** substitutes for picking a facilitator.

| | **`pay` CLI** (`@solana/pay`) | **pr402 buyer SDKs** | **CDP buyer libraries** |
|---|---|---|---|
| **Layer** | Buyer client | Buyer client (+ expects pr402 facilitator) | Buyer client (+ expects CDP facilitator) |
| **Typical use** | Universal wrapper for `curl`, Claude, Codex; Touch ID / MCP | `pr402-buy` CLI or embed `X402AgentClient` | Apps already on CDP stack |
| **Builds unsigned tx** | From 402 `accepts[]` locally (or seller-specific flow) | Via **`/build-exact-payment-tx`** / **`/build-sla-escrow-payment-tx`** on pr402 | Via CDP client patterns |
| **Works with pr402-backed APIs** | **Yes** — if seller settles via `ipay.sh` | **Yes** — native | Only if seller uses CDP, not pr402 |
| **Works with CDP-backed APIs** | Often yes (generic x402) | No — wrong facilitator shape | **Yes** — native |
| **`sla-escrow` on pr402** | Only if you implement escrow flow / follow seller 402 | **Yes** — first-class | N/A on Solana |
| **Debugger / sandbox** | **`debugger.pay.sh`**, `pay server demo`, `--dev` | **`preview.ipay.sh`** + [Buyer Quickstart](/quickstart-buyer.html) | CDP Devnet + API keys |
| **Official packages** | `@solana/pay`, `brew install pay` | `@pr402/client`, `pr402-client` (Rust) | `@coinbase/x402`, etc. |

**pr402 + `pay` CLI together:** A seller integrates **`ipay.sh`**. A buyer runs `pay curl https://seller.example/...` — the CLI handles 402 and signing; the **seller still forwards proof to pr402** for settle. Complementary layers.

---

## How the layers stack

```
┌─────────────────────────────────────────────────────────────┐
│  Buyer layer (pick one or more)                             │
│  pay CLI · @pr402/client · curl · custom agent              │
└───────────────────────────┬─────────────────────────────────┘
                            │ GET → 402 → sign → retry + PAYMENT-SIGNATURE
                            ▼
┌─────────────────────────────────────────────────────────────┐
│  Seller API (your server)                                   │
│  Return 402 · forward proof to facilitator · serve content  │
└───────────────────────────┬─────────────────────────────────┘
                            │ POST /verify · POST /settle
                            ▼
┌─────────────────────────────────────────────────────────────┐
│  Facilitator layer (seller chooses ONE per environment)     │
│  pr402 (ipay.sh) · CDP · x402.org · self-hosted            │
└───────────────────────────┬─────────────────────────────────┘
                            │ on-chain
                            ▼
              UniversalSettle (exact) · SLA-Escrow + oracles
```

- **Replacing the facilitator box:** pr402 ↔ CDP ↔ x402.org — this is the fair “which host?” decision.
- **`pay` CLI never replaces that box** — it only changes how the buyer reaches your 402.

---

## By persona

### Sellers (resource providers)

Compare **facilitators only** (Part A).

| Topic | **pr402** | **CDP** | **x402.org** |
|---|---|---|---|
| Integrate your API | **Core product** | Yes | Testnet only |
| Chain code in your server | **None** — 402 + `/settle` | Middleware patterns | Same idea |
| Canonical `payTo` | **SplitVault PDA** | Wallet-style on Solana `exact` | Wallet-style |
| Build 402 body | **`POST /upgrade`** | Framework middleware | Manual / samples |
| Escrow / SLA SKUs | **`sla-escrow`** | Not on Solana | Not available |
| Go-live path | **`preview.ipay.sh` → `ipay.sh`** | Devnet → Mainnet on CDP | Rewrite for production elsewhere |

Buyer CLI choice (`pay` vs `@pr402/client`) is **your customers’** decision — it does not change which facilitator **you** document in your 402 body.

### Buyers (payer agents)

Two-step decision:

1. **Read the seller’s 402** — which facilitator and rail (`exact` vs `sla-escrow`)?
2. **Pick a client** — Part B table.

For **pr402-backed** sellers: [Buyer Quickstart](/quickstart-buyer.html) or `@pr402/client`. Optionally wrap the same URLs with **`pay curl`** for Touch ID / MCP — settlement still happens on **`ipay.sh`** after the seller forwards proof.

### Oracles

| Topic | **pr402 ecosystem** | **CDP / x402.org** |
|---|---|---|
| On-chain escrow program | **`sla-escrow`** Mainnet + Devnet | No equivalent rail on Solana |
| Reference implementations | [`miraland-labs/oracles`](https://github.com/miraland-labs/oracles) | None as x402 escrow oracles |
| Facilitator discovery | `GET /capabilities → slaEscrowOracleProfiles[]` | N/A |

Oracle operators pair with the **pr402 facilitator layer**, not with `pay` CLI.

---

## Preview rehearsal (`preview.ipay.sh`)

Solana Devnet for x402 is **not** pr402-exclusive — CDP and x402.org support it too.

What **is** distinctive about **`preview.ipay.sh`** among **facilitators**:

| Capability | pr402 preview | Typical elsewhere |
|---|---|---|
| Paired hostname with production | `preview.ipay.sh` ↔ `ipay.sh` | Different URL or JSON `network` field |
| **`sla-escrow` on Devnet** | Yes | No |
| Seller **Preview / Activate** UI | Yes | No |
| Same OpenAPI + `/upgrade` + builders | Yes | Partial or `exact`-only |
| No account signup | Yes | CDP requires keys |

**`pay` CLI `--sandbox` / `debugger.pay.sh`** rehearses **buying** against demo APIs — not a substitute for **`preview.ipay.sh`** when you need to test **your seller integration** end-to-end on pr402.

---

## FAQ

### Why aren’t pr402, CDP, and pay.sh in one comparison row?

Because **`pay.sh` is not a facilitator**. A single row implies three settlement hosts. **`pay` CLI** is buyer-side; **CDP** and **pr402** are facilitator-side. Part A vs Part B keeps the comparison honest.

### Where is the facilitator if I use `pay curl`?

**Where the seller configured it.** The CLI signs and retries; the seller’s server POSTs proof to **their** facilitator (often `ipay.sh`, CDP, or x402.org). There is no `https://pay.sh/.../settle`.

### Can I use `pay` CLI to integrate my seller API?

**No.** Sellers integrate a **facilitator** (Part A). Buyers may use **`pay` CLI** (Part B) to call your API.

### Does pr402 compete with Coinbase on every axis?

**No.** CDP wins on multi-chain EVM, compliance, and breadth. pr402 wins on **Solana escrow**, **SplitVault seller economics**, **oracle ecosystem**, and **production-mirror preview**.

### Who should use x402.org instead?

Hackathons and **`exact`-only** Devnet spikes with zero signup. When you need Mainnet or **`sla-escrow`**, move to pr402 (or CDP for Solana `exact` only).

---

## Next steps

| Persona | Start here |
|---|---|
| Seller — pick pr402 facilitator | [Start here · Sellers](/start-here.html) |
| Buyer — pay pr402-backed API | [Buyer Quickstart](/quickstart-buyer.html) |
| Buyer — also want `pay` CLI | [`solana-foundation/pay`](https://github.com/solana-foundation/pay) + your seller’s facilitator docs |
| Oracle operator | [`oracles/` docs](https://github.com/miraland-labs/oracles) |
| Short differentiator list | [Start here · Appendix B](/start-here.html#appendix-b-why-pr402-vs-other-facilitators) |

**Live contract (pr402):** `GET https://ipay.sh/openapi.json` · `GET https://ipay.sh/api/v1/facilitator/capabilities`
