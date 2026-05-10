# API reference

This site is written for **human teams** (copy-friendly curl blocks, diagrams, checklists). **Autonomous agents** still need a single machine-readable contract against your facilitator host — that contract is **OpenAPI 3.1**.

## Humans — recommended reading order

1. [Buyer Quick Start](/quickstart-buyer) — six steps from HTTP 402 to `PAYMENT-SIGNATURE`.
2. [Seller Quick Start](/seller-quick-start) — 402 shape, `/settle`, language snippets.
3. [Seller shortcut (5 steps)](/quickstart-seller) — minimal path via `/upgrade`.
4. [Onboarding Guide](/onboarding_guide) — sovereign vs facilitated onboarding, fees, registry rules.
5. [Agent integration](/agent-integration) — full runbook (buyers, sellers, operational constraints).

Use this page when you need **schemas and endpoint names**; use the guides above when you need **intent and sequencing**.

## Agents — canonical artifacts

Always resolve against the **same facilitator origin** the seller documented (`$BASE`). Recommended defaults:

| Environment | Base URL |
|-------------|----------|
| Production (Solana Mainnet) | `https://ipay.sh` |
| Preview (Solana Devnet) | `https://preview.ipay.sh` |
| Alternate hostname (same service) | `https://agent.pay402.me` / `https://preview.agent.pay402.me` |

**Machine-readable API**

- **OpenAPI 3.1:** `{BASE}/openapi.json` — types, request/response bodies, examples (`X402V2VerifySettleBody`, builders, onboarding, etc.).
- **Markdown runbook (HTTP GET):** `{BASE}/agent-integration.md` — same narrative as [Agent integration](/agent-integration) when deployments stay in sync.

Confirm **`solanaNetwork`** (and related flags) with **`GET {BASE}/api/v1/facilitator/health`** or **`GET {BASE}/api/v1/facilitator/capabilities`** on the host you actually call.

### Using OpenAPI locally

Browsers often cannot fetch third-party JSON into hosted Swagger UIs because of CORS. Practical options:

```bash
curl -sS "https://ipay.sh/openapi.json" -o openapi.json
# Open openapi.json in your IDE’s OpenAPI tools, or paste into https://editor.swagger.io (manual upload).
```

For **preview**, substitute `https://preview.ipay.sh/openapi.json`.

## Endpoint map (high level)

| Area | Illustrative routes | Purpose |
|------|---------------------|---------|
| Health & discovery | `/api/v1/facilitator/health`, `/capabilities`, `/supported`, `/discovery` | Cluster, feature flags, rails |
| Buyer flows | `/build-exact-payment-tx`, `/build-sla-escrow-payment-tx`, `/verify`, `/settle` | Unsigned tx, proofs, settlement |
| Seller flows | `/upgrade`, `/onboard/*`, `/vault-snapshot` | 402 shaping, provisioning, ops |

Exact paths and bodies live **only** in **`openapi.json`** — avoid copying tables into offline cheat sheets when precision matters.

---

**Live contract:** behavior and feature flags can evolve; treat **`GET /capabilities`** and **`GET /openapi.json`** on the host you actually call as the source of truth.
