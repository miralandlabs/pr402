# Seller integration (pr402 facilitator)

The seller / resource-provider runbook is **[`public/onboarding_guide.md`](../public/onboarding_guide.md)** — static asset served at **`GET /onboarding_guide.md`** (same pattern as **`GET /openapi.json`**).

**Quick start:** [`public/quickstart-seller.md`](../public/quickstart-seller.md) · **VitePress:** [`docs-site/seller-quick-start.md`](../docs-site/seller-quick-start.md)

**Status:** pr402 is live on Solana Mainnet and Devnet; behavior and feature flags can evolve — `GET /capabilities` and `GET /openapi.json` on the host you call are the live contract.

| | **Production (Mainnet)** | **Preview (Devnet)** |
|--|--------------------------|----------------------|
| **Recommended** | `https://ipay.sh` | `https://preview.ipay.sh` |
| **Also (same APIs; not deprecated)** | `https://agent.pay402.me` | `https://preview.agent.pay402.me` |

Edit seller-facing Markdown under **`public/`** only. Confirm **`solanaNetwork`** on the host you integrate against: **`GET /api/v1/facilitator/health`**.

**Key seller HTTP paths** (see OpenAPI for bodies): `GET /sellers/{wallet}/preview`, `GET /sellers/{wallet}/rails/{scheme}`, `POST /sellers/provision-tx`, `GET /sellers/{wallet}/challenge`, `POST /sellers/{wallet}/register`, `POST /sellers/{wallet}/retire`, `POST /payment-required/enrich`, core x402 `verify` / `settle` proxy from your API.
