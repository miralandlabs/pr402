# SLA-Escrow full-cycle roadmap (API / SDK / audit)

This plan extends pr402 beyond **x402 `verify` / `settle`** (fund leg) so agents and resource providers can complete the **on-chain SLA-Escrow lifecycle** without ad-hoc shell CLIs. On-chain programs and **`sla-escrow`** CLI commands already exist; the gap is **productized HTTP + typed SDK**, **persistence**, and **signing / fee-payer consistency**.

**Related doc:** [sla_escrow_fee_payer_and_settle.md](./sla_escrow_fee_payer_and_settle.md) (fee payer vs `sign()` slot 0, facilitator-sponsored gas for FundPayment).

---

## 1. Problem statement

| Stage | On-chain | Today (typical) | Gap |
|-------|----------|------------------|-----|
| Fund | `FundPayment` | x402 `/verify`, `/settle` + `EscrowSdk` / CLI | Fee payer model may differ from `exact`; documented separately |
| Delivery | `SubmitDelivery` | `sla-escrow` CLI `submit-delivery` | No pr402 HTTP / shared SDK for apps |
| Oracle | `ConfirmOracle` | `sla-escrow` CLI `confirm-oracle` | Same |
| Release / refund | `ReleasePayment`, `RefundPayment` | CLI | Same |
| Housekeeping | `ExtendPaymentTTL`, `ClosePayment` | CLI | Same (lower priority for MVP) |

Without HTTP/SDK surfaces, **full cycle** for integrated products is **broken at the HTTP boundary**: only fund flows through the facilitator; everything else is “bring your own CLI.”

---

## 2. Goals

1. **Expose** post-fund instructions through a **stable API** (and optionally x402-flavored headers later), not only subprocess CLI.
2. Provide a **small Rust / TS SDK** (or OpenAPI-generated clients) that builds instructions / partially-signed txs where appropriate.
3. **Persist** lifecycle progress in **`escrow_details`** (and/or new tables) **per payment attempt**, not only per `escrow_pda` (see current upsert limitations).
4. Align **fee payer and signing** with product rules (facilitator vs seller vs oracle vs buyer) per instruction type.

---

## 3. Non-goals (initial phases)

- Reimplementing program logic off-chain.
- Replacing `sla-escrow-api` / `EscrowSdk` parsing; **reuse** and wrap it.
- Full x402 v2 spec for post-fund steps (unless you explicitly want `402` responses for each step).

---

## 4. Architecture options (pick one as baseline)

**A. Extend pr402 facilitator (monolith)**  
- New routes, e.g. `POST /api/v1/escrow/submit-delivery`, `.../confirm-oracle`, `.../release`, `.../refund`, under same deploy as verify/settle.  
- **Pros:** Single URL, shared config, RPC, DB. **Cons:** Larger surface; auth model must be clear.

**B. Separate “escrow-gateway” service**  
- Thin service: auth + build tx + optional submit; facilitator stays verify/settle-only.  
- **Pros:** Separation of concerns. **Cons:** Another deploy, shared secrets, correlation with `payment_attempts`.

**Recommendation:** Start with **A** for devnet + preview parity with existing `preview.pr402` host, feature-flagged routes; extract **B** later if blast radius or scaling demands it.

---

## 5. Phased roadmap

### Phase 0 — Prerequisites (no new routes)

- [x] **Data model:** Uniqueness is **`escrow_details(payment_attempt_id)`** (one row per payment). Migration: [`migrations/002_escrow_details_one_per_payment.sql`](../migrations/002_escrow_details_one_per_payment.sql); `upsert_escrow_detail` uses `ON CONFLICT (payment_attempt_id)`. Dashboards keyed **only** by `escrow_pda` now see **multiple** rows for the same mint rail—join via `payment_attempts` / `correlation_id` as needed.
- [ ] **Identity:** Standardize **correlation_id ↔ payment_uid** (string vs 32-byte sanitization already in chain layer) for all new APIs.
- [ ] **Auth:** Define who may call what:
  - **SubmitDelivery:** seller (payee) signing.
  - **ConfirmOracle:** oracle keypair from fund-time `oracle_authority` (or allowlist).
  - **Release / Refund:** per program state machine (seller / buyer / oracle outcome).
- [ ] **Fee payer matrix:** For each instruction, document and implement **who pays SOL** (align with [fee payer doc](./sla_escrow_fee_payer_and_settle.md)); avoid blind reuse of `sign()` on slot 0.

### Phase 1 — Build & sign helpers (SDK-first)

- [x] **`build-sla-escrow-payment-tx`:** `POST /api/v1/facilitator/build-sla-escrow-payment-tx` mirrors **`build-exact-payment-tx`** but produces an unsigned **`FundPayment`** shell (compute budget + optional escrow-vault ATA create + fund). **Buyer** is fee payer / signer — not the facilitator (see [fee payer doc](./sla_escrow_fee_payer_and_settle.md)). Instruction layout matches **`sla-escrow-api` / `EscrowSdk::fund_payment`**; PDAs use **`SLAEscrowConfig.program_id`** via [`sla_escrow_payment_build.rs`](../src/sla_escrow_payment_build.rs) + [`solana_sla_escrow.rs`](../src/chain/solana_sla_escrow.rs) (no CLI subprocess).
- [x] **Rust `pr402::sdk`:** Path constants (`BUILD_*_PAYMENT_TX_PATH`) + re-exported request/response types and `build_*_payment_tx` async functions.
- [x] **Examples:** `e2e_sign_sla_escrow_tx` (buyer signs bincode `VersionedTransaction` from the new endpoint).
- [x] **TS (optional):** [`sdk/facilitator-build-tx.ts`](../sdk/facilitator-build-tx.ts) — separate functions for **exact** vs **SLA-Escrow** paths (names avoid ambiguous “build-tx”).
- [ ] **Further split (optional):** Move instruction-only builders out of `pr402` into a tiny shared crate if `sla-escrow-cli` and facilitator both need identical imports without depending on full `pr402`.
- [ ] **Optional:** `POST .../build-submit-delivery-tx` returning unsigned/partially-signed `VersionedTransaction` + metadata (same pattern as `build-exact-payment-tx`).

### Phase 2 — Submit paths

Choose per route:

- **Relay mode:** Client signs full tx; POST body = base64 tx; server validates + `send_transaction` (similar to verify payload shape).
- **Facilitator co-sign mode:** Only where message layout uses facilitator as fee payer; server adds fee payer signature (mirror `exact`).

Deliver in order:

1. [ ] **SubmitDelivery** — body: `payment_uid` / `correlation_id`, `delivery_hash` (32 bytes hex), seller pubkey; return signature + update DB (`delivery_hash`, `delivery_signature` when known).
2. [ ] **ConfirmOracle** — body: `payment_uid`, `delivery_hash`, `resolution_state` (approve / reject); oracle must match fund-time authority; update `resolution_signature`, `resolution_state`.
3. [ ] **ReleasePayment** — after approved state; seller-facing; update `completed_at`, store tx sig.
4. [ ] **RefundPayment** — buyer-facing when program allows; update `refunded_at`.

### Phase 3 — Persistence & observability

- [x] Append-only **`escrow_lifecycle_events`** keyed by `payment_attempt_id` (migration [`003_escrow_lifecycle_events.sql`](../migrations/003_escrow_lifecycle_events.sql); mirrored in [`init.sql`](../migrations/init.sql)).
- [x] **`Pr402Db::apply_escrow_lifecycle_step`** merges lifecycle fields on `escrow_details` + inserts an event row (single transaction); CLI helper **`record_escrow_lifecycle`** for ops / E2E.
- [x] `server_log` tracing on apply path (correlation id, step, tx signature).
- [ ] Optional: webhook or polling hook for RP dashboards.

### Phase 4 — E2E & policy

- [x] Dual SLA fund E2E on devnet: **`03_sla_escrow_http_facilitator_fees.sh`** (facilitator fees) + **`01_sla_escrow_facilitator_verify.sh`** (CLI buyer-paid); orchestrated by **`run_all_devnet.sh`** (`SKIP_SLA_HTTP` / `SKIP_SLA_CLI`).
- [x] Shell E2E: [`scripts/e2e/04_sla_escrow_post_fund_lifecycle.sh`](../scripts/e2e/04_sla_escrow_post_fund_lifecycle.sh) after B1/B2 (submit-delivery → confirm-oracle → release or refund) + DB audit (`psql_audit_escrow_lifecycle`). Optional: `E2E_SLA_FULL_LIFECYCLE=1` on **B1** or `RUN_SLA_LIFECYCLE=1` in `run_all_devnet.sh` (chains after B1). Requires DB migration **003**, **`E2E_ORACLE_KEYPAIR`** matching fund-time oracle, and `DATABASE_URL`.
- [x] **Single-shot full cycle:** [`scripts/e2e/05_sla_escrow_full_cycle_devnet.sh`](../scripts/e2e/05_sla_escrow_full_cycle_devnet.sh) sets `E2E_ORACLE_AUTHORITY` from `E2E_ORACLE_KEYPAIR` (default buyer), patches B1 `extra.oracleAuthorities` for `/verify`, runs B1 + chained **04**. **`RUN_FULL_SLA_LIFECYCLE=1`** in `run_all_devnet.sh` runs **05** instead of B2+B1.
- [ ] Document security: never accept oracle calls without checking on-chain payment state + pubkey.

### Phase 5 — FundPayment fee payer (optional product alignment)

- [x] **Facilitator fee payer (default):** `build-sla-escrow-payment-tx` uses facilitator as message fee payer (two signers); `/verify` + `/settle` align with `exact` (`sign` slot 0 + `settle_transaction`). **`buyerPaysTransactionFees: true`** preserves buyer-paid / CLI-shaped txs (`settle_sla_escrow_fund_payment`). See [sla_escrow_fee_payer_and_settle.md](./sla_escrow_fee_payer_and_settle.md), `/supported` `extra.slaFundTxNetworkFeePayer`.
- [ ] **CLI:** Optional non-send / facilitator fee-payer build mode (or document HTTP-only); `fund-payment` `about` text points agents at pr402.

---

## 6. Instruction → actor → suggested API sketch

| Instruction | Primary signer(s) | Fee payer (target design) | Example route |
|-------------|-------------------|---------------------------|---------------|
| `SubmitDelivery` | Seller | Seller or facilitator (TBD) | `POST /api/v1/escrow/submit-delivery` |
| `ConfirmOracle` | Oracle | Oracle or facilitator | `POST /api/v1/escrow/confirm-oracle` |
| `ReleasePayment` | Seller (typical) | Same matrix | `POST /api/v1/escrow/release-payment` |
| `RefundPayment` | Buyer (typical) | Same matrix | `POST /api/v1/escrow/refund-payment` |
| `ExtendPaymentTTL` | Buyer/seller per program | TBD | lower priority |
| `ClosePayment` | Cleanup | TBD | lower priority |

Exact account lists remain **source of truth** in `sla-escrow` program + `EscrowSdk`; API only orchestrates.

---

## 7. Open decisions (capture before Phase 1 coding)

1. **Monolith vs gateway** (§4).
2. **Per-payment `escrow_details` vs event log** (§5 Phase 0).
3. **Authentication:** API keys, JWT, or raw Solana signatures only?
4. **TS SDK** ownership (frontend team vs same crate as pr402).

---

## 8. Success criteria

- An integrator can drive **fund → delivery → oracle → release** using **HTTP + one SDK** without `sla-escrow` binary on PATH.
- DB reflects lifecycle fields (**delivery_hash**, **resolution_state**, **completed_at** / **refunded_at**) for a single **payment_attempt** traceably.
- E2E proves the path on devnet against preview (or dedicated stack).

---

*This is a planning document; implementation should be split into separate PRs per phase.*
