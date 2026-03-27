# pr402: x402 Facilitator for Solana

**A minimal, x402-compliant facilitator for Solana, optimized for Vercel Serverless Functions. It bridges human/agent requests with on-chain settlement engines like `UniversalSettle` and `SLA-Escrow`.**

## 🧩 Protocol Overview
This facilitator implements a tailored version of the [x402-rs](https://github.com/x402-rs/x402-rs) protocol, supporting:
- ✅ **Solana-Only**: High-performance, lightweight implementation with no dependencies on multi-chain libraries.
- ✅ **Protocol v2**: Latest protocol version for agentic economies.
- ✅ **Multi-Scheme Settlement**: Native support for "Exact" and "Escrow" payment schemes.
- ✅ **Vercel Serverless Functions**: Optimized for low-latency, stateless API endpoints.

## 🛠️ Supported Schemes
pr402 currently facilitates two core settlement patterns:

1.  **`v2:solana:exact` (UniversalSettle)**: Used for immediate settlement (e.g., direct payments, subscriptions, one-time fees). Features the **Vault Triple** pattern with 0-data SOL storage.
2.  **`v2:solana:sla-escrow` (SLA-Escrow)**: Used for conditional settlement where payment is released upon Service Level Agreement (SLA) fulfillment. Also uses 0-data SOL storage.

## 📁 Project Structure
- [`src/bin/facilitator.rs`](src/bin/facilitator.rs) — Vercel serverless entrypoint handling HTTP requests.
- [`src/chain/`](src/chain/) — Solana-specific chain provider and instruction builders for UniversalSettle and SLA-Escrow.
- [`src/scheme/`](src/scheme/) — Protocol verification logic for Exact and Escrow schemes.
- [`src/config.rs`](src/config.rs) — Environment-based configuration.

## ⚙️ Environment Variables
Required:
- `SOLANA_RPC_URL`: Solana RPC endpoint.
- `SOLANA_CHAIN_ID`: Chain ID in CAIP-2 format (e.g., `solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`).
- `FEE_PAYER_PRIVATE_KEY`: Base58-encoded private key for the facilitator's fee payer.

Optional:
- `UNIVERSALSETTLE_PROGRAM_ID`: Program ID for UniversalSettle (activates `v2:solana:exact`).
- `ESCROW_PROGRAM_ID`: Program ID for SLA-Escrow (activates `v2:solana:sla-escrow`).
- `MAX_COMPUTE_UNIT_LIMIT`: Transaction compute limit (default: 400,000).
- `MAX_COMPUTE_UNIT_PRICE`: Transaction compute price in micro-lamports (default: 1,000,000).

## 🚀 API Endpoints
When deployed, the facilitator exposes:
- `POST /api/facilitator/verify` — Verify raw payment transactions against the protocol.
- `POST /api/facilitator/settle` — Sign and relay verified transactions to the Solana network.
- `GET /api/facilitator/supported` — List active schemes based on environment configuration.
- `GET /api/facilitator/health` — Same handler as `supported` (load balancer / uptime check).

### Vercel deployment
- **`vercel.json`** uses `vercel-rust@4.0.8` and routes `/api/facilitator/*` to the `facilitator` binary.
- CI deploy: **[`.github/workflows/build-and-deploy.yml`](.github/workflows/build-and-deploy.yml)** (repository root = this project).
- In the Vercel project settings, **Root Directory** should be the repo root (or leave blank), not a parent monorepo path — otherwise you can get a platform **404** for `/api/facilitator/health`.

### Correlation id (optional DB merge key)
x402 does not require a correlation id. For integrators who **do** enable Postgres (`DATABASE_URL`), pr402 merges `/verify` and `/settle` into one `payment_attempts` row when the **same** id is used.

**Easiest path (no id in the request):** On **successful** `/verify`, if the body includes `paymentRequirements.payTo` and the request omits `correlationId` / `X-Correlation-Id`, the facilitator **mints** a ULID, persists the verify outcome, and returns it as **`correlationId`** in the JSON body and **`X-Correlation-Id`** on the response. Re-send that value on **`/settle`** (same header or `correlationId` in JSON) so settlement updates the same row.

**Bring your own id:** Set `correlationId` or `X-Correlation-Id` on both calls as before; server minting is skipped.

---
Part of the **x402 Agentic Protocol** ecosystem.
