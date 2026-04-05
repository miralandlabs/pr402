# pr402: x402 Facilitator for Solana

**A minimal, x402-compliant facilitator for Solana, optimized for Vercel Serverless Functions. It bridges human/agent requests with on-chain settlement engines like `UniversalSettle` and `SLA-Escrow`.**

## For resource provider agents (sellers)

Sellers can onboard either **Proactively** (Protocol Onboarding) to receive a fee discount, or **Just-In-Time** (Facilitated) with a small setup recovery fee.

**Agentic Onboarding Flow:**
1. **Discover**: Check status at `GET /api/v1/facilitator/onboard?wallet=<PUBKEY>`.
2. **On-Chain Provisioning**: Build a sovereign vault tx via `GET /api/v1/facilitator/onboard/build-tx?wallet=<PUBKEY>`. Sign & Send locally to receive the 95 bps rate.
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

## 🚀 API Endpoints (v1)
- **OpenAPI 3.1:** [`public/openapi.json`](public/openapi.json) — served at **`GET /openapi.json`** on the deployed host (and `GET /api/v1/facilitator/openapi.json` redirects there). Use it for agents, codegen, and contract tests. `GET /api/v1/facilitator/capabilities` includes `httpEndpoints.openApi` pointing to this path.
- **Agent runbook:** edit [`public/agent-integration.md`](public/agent-integration.md) (static, like **`public/openapi.json`**); served at **`GET /agent-integration.md`** (`vercel.json` route). [`docs/AGENT_INTEGRATION.md`](docs/AGENT_INTEGRATION.md). Listed under `capabilities.httpEndpoints.agentIntegration`.
- `GET /api/v1/facilitator/supported`: Returns the **Enriched Metadata** for all active schemes. This is the primary discovery endpoint for AI Agents.
- `POST /api/v1/facilitator/verify`: Validates transactions against the protocol requirements and the agent's selected oracle.
- `POST /api/v1/facilitator/settle`: Relays the signed transaction to the blockchain.
- `GET /api/v1/facilitator/health` — Same handler as `supported` (load balancer / uptime check).
- `GET /api/v1/facilitator/capabilities` — Discovery JSON: `chainId`, `feePayer`, `supported` kinds, feature flags (UniversalSettle, escrow, unsigned tx build, buildOnboardTx), and relative HTTP endpoint paths.
- **Onboarding (Sellers)**:
    - `GET /api/v1/facilitator/onboard?wallet=...` — Preview current status and "Sovereign" eligibility.
    - `GET /api/v1/facilitator/onboard/build-tx?wallet=...` — Build an unsigned `create_vault` tx for proactive agents.
    - `GET /api/v1/facilitator/onboard/challenge?wallet=...` — Receive a unique message for DB registration.
    - `POST /api/v1/facilitator/onboard` — Submit checked challenge to persist seller metadata.
- `POST /api/v1/facilitator/build-exact-payment-tx` — Build an **unsigned** SPL payment transaction (compute budget + optional merchant ATA create + `TransferChecked`) matching one `accepts[]` line. Body: `{ "payer", "accepted", "resource", "skipSourceBalanceCheck"? }`. Response includes `transaction` (base64 bincode) and `verify_body_template` (replace `paymentPayload.payload.transaction` after the payer signs). Same layout as the local `x402_pr402_pay` helper in spl-token-balance-serverless; **native SOL** mint is rejected here (use a different path). **Who calls it:** the **buyer** (wallet, browser, or agent) over HTTPS — not the resource provider; RP only issues the `402` and `accepts[]`. **Why “shared”:** one facilitator implements this for **all** RPs on that deployment (RPs are not required to host Solana tx construction). **CORS:** `OPTIONS /api/v1/facilitator/*` returns **204** with `Access-Control-Allow-*`; JSON responses include `Access-Control-Allow-Origin: *`.
- `POST /api/v1/facilitator/build-sla-escrow-payment-tx` — Build an **unsigned** SLA-Escrow **`FundPayment`** transaction (compute budget + optional escrow vault ATA create + fund). Body: `payer`, `accepted` (`scheme: "sla-escrow"`), `resource`, **`slaHash`** (64 hex), **`oracleAuthority`**, optional `paymentUid`, optional `skipSourceBalanceCheck`, optional **`buyerPaysTransactionFees`** (default `false`). **Default:** facilitator pays Solana fees (same signer layout as `build-exact-payment-tx`; buyer partially signs, facilitator completes at `/settle`). **`buyerPaysTransactionFees: true`:** buyer fee payer / one signer (CLI-compatible). Requires `ESCROW_PROGRAM_ID` / SLA escrow config. Classic SPL Token mints only in this builder (Token-2022 and native SOL fund layouts: use `sla-escrow` CLI or extend the builder).

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
