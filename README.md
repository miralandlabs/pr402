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

pr402 currently facilitates two core settlement patterns in accordance with the x402 V2 standard:

1.  **`exact` (UniversalSettle)**: Used for high-velocity, immediate settlement. 
    - **Enriched Metadata**: Discloses `programId`, `configAddress`, and `feeBps`.
2.  **`sla-escrow` (SLA-Escrow)**: Used for high-stakes or conditional settlement.
    - **Enriched Metadata**: Discloses `escrowProgramId`, `bankAddress`, `configAddress`, `feeBps`, and `oracleAuthorities`.

---

## 📁 Project Structure
- [`src/bin/facilitator.rs`](src/bin/facilitator.rs) — Vercel serverless entrypoint handling HTTP requests.
- [`src/chain/`](src/chain/) — Solana-specific chain provider and instruction builders for UniversalSettle and SLA-Escrow.
- [`src/scheme/`](src/scheme/) — Protocol verification logic for Exact and Escrow schemes.
- [`src/exact_payment_build.rs`](src/exact_payment_build.rs) — Optional **shared** SPL `TransferChecked` tx shell for `v2:solana:exact` (unsigned legacy `VersionedTransaction` + verify-body template).
- [`src/config.rs`](src/config.rs) — Environment-based configuration.

## ⚙️ Environment Variables
Required:
- `SOLANA_RPC_URL`: Solana RPC endpoint.
- `SOLANA_CHAIN_ID`: Chain ID in CAIP-2 format.
- `FEE_PAYER_PRIVATE_KEY`: Base58 private key for the facilitator.

Scheme Configuration:
- `UNIVERSALSETTLE_PROGRAM_ID`: Program ID for UniversalSettle.
- `ESCROW_PROGRAM_ID`: Program ID for SLAEscrow.
- `ORACLE_AUTHORITIES`: Comma-separated list of trusted Oracle pubkeys advertised as candidates during discovery.

---

## 🛡️ Reliability & Security Standard
To ensure the highest level of transparency for the Agentic Economy, the facilitator implements the following standards:

- **Bit-Perfect Discovery**: On-chain state is extracted using **8-byte discriminators** (Anchor-compatible) and the authoritative protocol API structs. This ensures that the metadata advertised to agents matches the on-chain reality with 100% bit-level precision.
- **Pluralistic Trust**: By advertising multiple `oracleAuthorities`, the facilitator allows buyer agents to autonomously select the most trusted candidate for their specific task.
- **Fail-Fast Registration**: The facilitator strictly validates both scheme configurations at startup, returning explicit errors if on-chain properties cannot be loaded.

---

## 🚀 API Endpoints (v1)
- `GET /api/v1/facilitator/supported`: Returns the **Enriched Metadata** for all active schemes. This is the primary discovery endpoint for AI Agents.
- `POST /api/v1/facilitator/verify`: Validates transactions against the protocol requirements and the agent's selected oracle.
- `POST /api/v1/facilitator/settle`: Relays the signed transaction to the blockchain.
- `GET /api/v1/facilitator/health` — Same handler as `supported` (load balancer / uptime check).
- `GET /api/v1/facilitator/capabilities` — Discovery JSON: `chainId`, `feePayer`, `supported` kinds, feature flags (UniversalSettle, escrow, unsigned tx build), and relative HTTP endpoint paths + x402 v2 spec link.
- `POST /api/v1/facilitator/build-exact-payment-tx` — Build an **unsigned** SPL payment transaction (compute budget + optional merchant ATA create + `TransferChecked`) matching one `accepts[]` line. Body: `{ "payer", "accepted", "resource", "skipSourceBalanceCheck"? }`. Response includes `transaction` (base64 bincode) and `verify_body_template` (replace `paymentPayload.payload.transaction` after the payer signs). Same layout as the local `x402_pr402_pay` helper in spl-token-balance-serverless; **native SOL** mint is rejected here (use a different path). **Who calls it:** the **buyer** (wallet, browser, or agent) over HTTPS — not the resource provider; RP only issues the `402` and `accepts[]`. **Why “shared”:** one facilitator implements this for **all** RPs on that deployment (RPs are not required to host Solana tx construction). **CORS:** `OPTIONS /api/v1/facilitator/*` returns **204** with `Access-Control-Allow-*`; JSON responses include `Access-Control-Allow-Origin: *`.
- `POST /api/v1/facilitator/build-sla-escrow-payment-tx` — Build an **unsigned** SLA-Escrow **`FundPayment`** transaction (compute budget + optional escrow vault ATA create + fund). Body: `payer`, `accepted` (`scheme: "sla-escrow"`), `resource`, **`slaHash`** (64 hex), **`oracleAuthority`**, optional `paymentUid`, optional `skipSourceBalanceCheck`, optional **`buyerPaysTransactionFees`** (default `false`). **Default:** facilitator pays Solana fees (same signer layout as `build-exact-payment-tx`; buyer partially signs, facilitator completes at `/settle`). **`buyerPaysTransactionFees: true`:** buyer fee payer / one signer (CLI-compatible). Requires `ESCROW_PROGRAM_ID` / SLA escrow config. Classic SPL Token mints only in this builder (Token-2022 and native SOL fund layouts: use `sla-escrow` CLI or extend the builder).

### Vercel deployment
- **`vercel.json`** uses `vercel-rust@4.0.8` and maps each `/api/v1/facilitator/...` path to the `facilitator` binary.
- CI deploy: **[`.github/workflows/build-and-deploy.yml`](.github/workflows/build-and-deploy.yml)** (repository root = this project).
- In the Vercel project settings, **Root Directory** should be the repo root (or leave blank), not a parent monorepo path — otherwise you can get a platform **404** for `/api/v1/facilitator/health`.

### Correlation id (optional DB merge key)
x402 does not require a correlation id. For integrators who **do** enable Postgres (`DATABASE_URL`), pr402 merges `/verify` and `/settle` into one `payment_attempts` row when the **same** id is used.

**Easiest path (no id in the request):** On **successful** `/verify`, if the body includes `paymentRequirements.payTo` and the request omits `correlationId` / `X-Correlation-Id`, the facilitator **mints** a ULID, persists the verify outcome, and returns it as **`correlationId`** in the JSON body and **`X-Correlation-Id`** on the response. Re-send that value on **`/settle`** (same header or `correlationId` in JSON) so settlement updates the same row.

**Bring your own id:** Set `correlationId` or `X-Correlation-Id` on both calls as before; server minting is skipped.

---
Part of the **x402 Agentic Protocol** ecosystem.
