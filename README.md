# 🌉 pr402: x402 Facilitator for Solana

**pr402** is the REST-to-Solana gateway for the x402 agentic economy, optimized for serverless environments (Vercel). It translates simple REST API requests from off-chain agents into on-chain instructions for `UniversalSettle` and `SLA-Escrow`.

---

## 📌 Official Deployments

Integrations must use the origin configured for that environment. Confirm the cluster and capability parameters via **`GET /api/v1/facilitator/health`** on your host.

| Environment | Primary Origin (Concise) | Alternate Host (Same Service) | OpenAPI Schema |
| :--- | :--- | :--- | :--- |
| **Production** | `https://ipay.sh` | `https://agent.pay402.me` | [`/openapi.json`](https://ipay.sh/openapi.json) |
| **Preview** | `https://preview.ipay.sh` | `https://preview.agent.pay402.me` | [`/openapi.json`](https://preview.ipay.sh/openapi.json) |

---

## 🛠️ API Surface Overview

All endpoints are detailed in the `openapi.json` schema. The primary endpoints are:

* **`/capabilities` (GET):** Returns the facilitator's supported rails, networks, default oracles, and fee rates.
* **`/payment-required/enrich` (POST):** Enriches a draft payment required body into an institutional-grade, client-ready 402 template.
* **`/build-exact-payment-tx` (POST):** Builds an unsigned `VersionedTransaction` for the `exact` rail.
* **`/build-sla-escrow-payment-tx` (POST):** Builds an unsigned `VersionedTransaction` for the `sla-escrow` rail.
* **`/verify` (POST):** Dry-runs payment signature validation.
* **`/settle` (POST):** Validates and broadcasts the transaction on-chain, completing settlement.

---

## 🚀 Developer Integration Paths

Detailed guides are served directly by the deployments or can be found in the workspace:

* **Sellers (API Gating):** Read the [Start here · Sellers](/start-here.html) guide or checkout [x402-seller-starter](https://github.com/miraland-labs/x402-seller-starter).
* **Buyers (Agents):** Install the client SDK (`npm i @pr402/client` or `cargo install pr402-client`) and refer to the [Buyer Quickstart](docs-site/quickstart-buyer.md).
* **AI/MCP Integration:** Run `npx -y @pr402/mcp-server` to connect tools directly to LLMs or Cursor.

---

## ⚙️ Environment Configuration

Deployments require the following environment variables:

| Variable | Description |
| :--- | :--- |
| `SOLANA_RPC_URL` | Solana RPC node provider URL. |
| `SOLANA_CHAIN_ID` | CAIP-2 chain identifier. |
| `FEE_PAYER_PRIVATE_KEY` | Base58 private key of the facilitator (pays gas for JIT vaults and sweeps). |
| `UNIVERSALSETTLE_PROGRAM_ID` | Program ID of the UniversalSettle program. |
| `ESCROW_PROGRAM_ID` | Program ID of the SLA-Escrow program. |
| `ORACLE_AUTHORITIES` | Comma-separated list of trusted oracle public keys. |
