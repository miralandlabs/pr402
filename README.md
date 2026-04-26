# pr402: x402 Facilitator for Solana

**A minimal, x402-compliant facilitator for Solana, optimized for Vercel Serverless Functions. It bridges human/agent requests with on-chain settlement engines like `UniversalSettle` and `SLA-Escrow`.**

> **Launch phase:** pr402 is **experimental**. Use **at your own risk** — behavior, fees, allowlists, and availability may change without notice.

## Official deployments

| Environment | Base URL | Solana |
|-------------|----------|--------|
| **Production** | `https://agent.pay402.me` | Mainnet |
| **Preview** | `https://preview.agent.pay402.me` | Devnet |

Call **`GET /api/v1/facilitator/health`** or **`GET /api/v1/facilitator/capabilities`** on the **same host** you use for `verify` / `settle` to confirm **`solanaNetwork`**, **`chainId`**, and feature flags. Integrations must use the origin the **seller documents** for that resource (do not silently swap preview vs production).

**Wallet RPC:** If your client needs the deployment’s wallet-facing HTTP RPC, read **`solanaWalletRpcUrl`** from **`GET /health`** at runtime. Do not copy RPC URLs from static markdown into apps — they are environment-specific and may rotate or carry credentials.

| Served doc | Production | Preview |
|------------|------------|---------|
| OpenAPI 3.1 | [`/openapi.json`](https://agent.pay402.me/openapi.json) | [`/openapi.json`](https://preview.agent.pay402.me/openapi.json) |
| Buyer runbook | [`/agent-integration.md`](https://agent.pay402.me/agent-integration.md) | [`/agent-integration.md`](https://preview.agent.pay402.me/agent-integration.md) |
| Seller onboarding | [`/onboarding_guide.md`](https://agent.pay402.me/onboarding_guide.md) | [`/onboarding_guide.md`](https://preview.agent.pay402.me/onboarding_guide.md) |

In-repo copies: [`public/openapi.json`](public/openapi.json), [`public/agent-integration.md`](public/agent-integration.md), [`public/onboarding_guide.md`](public/onboarding_guide.md).

## For resource provider agents (sellers)

Sellers can onboard either **Proactively** (Protocol Onboarding) to receive a fee discount, or **Just-In-Time** (Facilitated) with a small setup recovery fee.

**Agentic Onboarding Flow:**
1. **Discover**: Find your `payTo` (vault PDA) address via `GET /api/v1/facilitator/discovery?wallet=<PUBKEY>&scheme=exact`.
2. **On-Chain Provisioning**: **`POST /api/v1/facilitator/onboard/provision`** with a **JSON body** (`Content-Type: application/json`). Put `wallet` and `asset` **in the request body**, not in the query string — e.g. `{"wallet":"<PUBKEY>","asset":"SOL"}` (`USDC`, `WSOL`, `USDT`, or a base58 mint). Example: `curl -X POST .../onboard/provision -H "Content-Type: application/json" -d '{"wallet":"<PUBKEY>","asset":"SOL"}'`. Idempotent when already on-chain; sign & broadcast when `transaction` is present. (Launch-phase policy: **one asset per merchant wallet**; use **separate wallets** for additional tokens — see [`public/onboarding_guide.md`](public/onboarding_guide.md).)
3. **Registry (Off-Chain)**: Use `/onboard/challenge` to persist verified metadata for high-fidelity discovery.

## For buyer agents (payers)

This section is for **buyer-side** code: wallets, automation, and agents that **pay** after a resource returns **HTTP 402** with `accepts[]`. Resource providers only return the challenge; they do not normally call the build endpoints below.

### End-to-end flow

1. **Receive** the `402` body from the resource: keep `paymentRequirements` and pick one matching `accepts[]` line.
2. **Discover** your facilitator (same host the RP referenced, if any): `GET /api/v1/facilitator/capabilities` or `GET /api/v1/facilitator/supported`. Use `httpEndpoints` + `GET /openapi.json` for the machine-readable contract.
3. **Build** (optional): if the RP relies on this facilitator for tx assembly, call **`POST .../build-exact-payment-tx`** when `scheme` is `exact` or `v2:solana:exact`, or **`POST .../build-sla-escrow-payment-tx`** when `scheme` is `sla-escrow` or `v2:solana:sla-escrow` — not both. You send `payer`, `accepted`, `resource`, and (escrow only) `slaHash` + `oracleAuthority`.
4. **Sign** the unsigned `transaction` (base64 bincode) with the payer’s Solana signer, then put the signed bytes back into `paymentPayload.payload.transaction` inside the `verifyBodyTemplate` from the build response (see OpenAPI / runbook).
5. **Verify** then **settle**: `POST .../verify` and `POST .../settle` with the **same** JSON body; reuse `correlationId` / `X-Correlation-ID` if the facilitator returned one on verify.

Rebuild unsigned tx (`retry build` error) if signing or retry is delayed and blockhash expires. If the RP already gave a fully built fund tx (some escrow CLI flows), skip step 3 and still use steps 4–5.

### SDKs and docs

| Integration | What to use |
|-------------|-------------|
| **Step-by-step** | **[`public/agent-integration.md`](public/agent-integration.md)** (same static pattern as `openapi.json`; **`GET /agent-integration.md`**). Stub: [`docs/AGENT_INTEGRATION.md`](docs/AGENT_INTEGRATION.md). |
| **Schema / codegen** | **`GET /openapi.json`** on the facilitator base URL (see `capabilities.httpEndpoints.openApi`). |
| **TypeScript** | Copy or import [`sdk/facilitator-build-tx.ts`](sdk/facilitator-build-tx.ts): `getCapabilities`, `buildExactPaymentTx`, `verifyPayment`, `settlePayment`, etc. (`fetch` only). |
| **Rust** | Add **`pr402`** with feature **`facilitator-http`**, then use **`pr402::sdk::http`**: [`FacilitatorHttpClient`](src/sdk/http.rs) or the free async functions (same paths as the TS file). Omit this feature when **deploying** the `facilitator` binary. |
| **Other stacks** | Call the same HTTPS paths; bodies match OpenAPI (`BuildExactPaymentTxRequest`, `X402V2VerifySettleBody`, …). |

## 🧩 Protocol Overview
This facilitator implements a tailored version of the [x402-rs](https://github.com/x402-rs/x402-rs) protocol, supporting:
- ✅ **Solana-Only**: High-performance, lightweight implementation with no dependencies on multi-chain libraries.
- ✅ **Protocol v2**: Latest protocol version for agentic economies.
- ✅ **Multi-Scheme Settlement**: Native support for "Exact" and "Escrow" payment schemes.
- ✅ **Agent-Native Onboarding**: Proactive, machine-readable onboarding for sovereign status.
- ✅ **Vercel Serverless Functions**: Optimized for low-latency, stateless API endpoints.

## 🛠️ Supported Schemes
Two settlement patterns (x402 v2):

1.  **`exact` (UniversalSettle)**: Used for high-velocity, immediate settlement. 
    - **Enriched Metadata**: Discloses `programId`, `configAddress`, and `feeBps`.
2.  **`sla-escrow` (SLA-Escrow)**: Used for high-stakes or conditional settlement.
    - **Enriched Metadata**: Discloses `escrowProgramId`, `bankAddress`, `configAddress`, `feeBps`, and `oracleAuthorities`.

---

## 📁 Project Structure
- **Buyer SDKs:** see [For buyer agents](#for-buyer-agents-payers); TS [`sdk/facilitator-build-tx.ts`](sdk/facilitator-build-tx.ts), Rust [`src/sdk/http.rs`](src/sdk/http.rs) (`facilitator-http` feature).
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

## 🛡️ Institutional Identity Standard
In the UniversalSettle and SLA-Escrow ecosystems, the concept of a "beneficiary" is split to ensure buyer agents can be stateless:

1.  **`payTo` (The Destination)**: This is MUST be the direct on-chain destination for the transaction. For institutional payments, this is the **SplitVault PDA** or **Escrow Bank PDA**.
2.  **`extra.merchantWallet` (The Identity)**: This is the original merchant wallet. The Facilitator uses this to re-derive PDAs for fee-sweeping and provisioning. 

**Standard Compliance Rule**: Buyers SHOULD always pay the address in `payTo`. Facilitators MUST look for the merchant's identity in `extra.merchantWallet` if `payTo` is a PDA.

---

## 🛡️ Reliability & Security Standard
To ensure the highest level of transparency for the Agentic Economy, the facilitator implements the following standards:

- **Bit-Perfect Discovery**: On-chain state is extracted using **8-byte discriminators** (Anchor-compatible) and the authoritative protocol API structs. This ensures that the metadata advertised to agents matches the on-chain reality with 100% bit-level precision.
- **Pluralistic Trust**: By advertising multiple `oracleAuthorities`, the facilitator allows buyer agents to autonomously select the most trusted candidate for their specific task.
- **Fail-Fast Registration**: The facilitator strictly validates both scheme configurations at startup, returning explicit errors if on-chain properties cannot be loaded.

---

## API surface (v1)

The **authoritative** list of paths, request bodies, and schemas is **[`public/openapi.json`](public/openapi.json)** (`GET /openapi.json` on each deployment). **`GET /api/v1/facilitator/capabilities`** returns relative paths and `httpEndpoints.openApi`.

**Core x402 path:** `supported` → optional **`build-exact-payment-tx`** or **`build-sla-escrow-payment-tx`** → **`verify`** → **`settle`** (same JSON body for verify/settle; see [`public/agent-integration.md`](public/agent-integration.md)).

**Discovery & ops:** `health`, `capabilities`, `discovery`, seller `onboard/*`, `upgrade`, operator sweep/snapshot (OpenAPI tag **Operations**).

### Vercel deployment
- **`vercel.json`** uses `vercel-rust@4.0.8` and maps each `/api/v1/facilitator/...` path to the `facilitator` binary.
- CI deploy: **[`.github/workflows/build-and-deploy.yml`](.github/workflows/build-and-deploy.yml)** (repository root = this project).
- In the Vercel project settings, **Root Directory** should be the repo root (or leave blank), not a parent monorepo path — otherwise you can get a platform **404** for `/api/v1/facilitator/health`.

### Correlation id (optional DB merge key)
x402 does not require a correlation id. For integrators who **do** enable Postgres (`DATABASE_URL`), pr402 merges `/verify` and `/settle` into one `payment_attempts` row when the **same** id is used.

**Easiest path (no id in the request):** On **successful** `/verify`, if the body includes `paymentRequirements.payTo` and the request omits `correlationId` / `X-Correlation-ID`, the facilitator **mints** a ULID, persists the verify outcome, and returns it as **`correlationId`** in the JSON body and **`X-Correlation-ID`** on the response. Re-send that value on **`/settle`** (same header or `correlationId` in JSON) so settlement updates the same row.

**Bring your own id:** Set `correlationId` or `X-Correlation-ID` on both calls as before; server minting is skipped.

---
Part of the **x402 Agentic Protocol** ecosystem.
