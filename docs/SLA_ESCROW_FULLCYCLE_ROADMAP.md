# SLA-Escrow full-cycle roadmap (API / SDK / audit)

Planning doc: what pr402 ships today vs what remains for **post-fund** lifecycle (delivery → oracle → release/refund) beyond x402 **`verify` / `settle`**.

**Related:** [SLA_ESCROW_FEE_PAYER_AND_SETTLE.md](./SLA_ESCROW_FEE_PAYER_AND_SETTLE.md) · [CRON_OPERATIONS.md](./CRON_OPERATIONS.md) · [OPS_RECOVERY_PLAYBOOK.md](./OPS_RECOVERY_PLAYBOOK.md)

On-chain programs and **`sla-escrow` CLI** already implement all instructions. The product gap is **HTTP/SDK surfaces** and **operator automation** for steps after fund.

---

## 1. Lifecycle stages

| Stage | On-chain | Shipped in pr402 HTTP | Gap |
|-------|----------|----------------------|-----|
| Fund | `FundPayment` | `/verify`, `/settle`, `build-sla-escrow-payment-tx` | — |
| Delivery | `SubmitDelivery` | — | CLI / direct program only |
| Oracle | `ConfirmOracle` | — | CLI / oracle operator |
| Release / refund (post-outcome) | `ReleasePayment`, `RefundPayment` | Settlement keeper cron + `build-sla-escrow-settle-tx` | No HTTP builder for post-fund instructions |
| Housekeeping | `ExtendPaymentTTL`, `ClosePayment` | `sla-escrow-close-cron` (+ `POST /sla-escrow-close`) | Lower priority |

---

## 2. Shipped (verified against `src/bin/facilitator.rs`)

### Fund leg (x402)

- [x] `POST /api/v1/facilitator/build-sla-escrow-payment-tx` — unsigned FundPayment shell (`src/sla_escrow_payment_build.rs`).
- [x] Default **buyer** fee payer; optional **facilitator** fee payer via `facilitatorPaysTransactionFees: true` when `PR402_SLA_ESCROW_ALLOW_FACILITATOR_FEE_SPONSORSHIP` is enabled.
- [x] `/verify` + `/settle` with dual settle paths (see [SLA_ESCROW_FEE_PAYER_AND_SETTLE.md](./SLA_ESCROW_FEE_PAYER_AND_SETTLE.md)).
- [x] Rust SDK path constants + HTTP helpers (`src/sdk/`).
- [x] TS reference: `sdk/facilitator-build-tx.ts`.

### Persistence

- [x] `escrow_details` one row per `payment_attempt_id` (`migrations/002_escrow_details_one_per_payment.sql`).
- [x] `payment_uid_hex` column (`migrations/007_escrow_payment_uid.sql`).
- [x] `escrow_lifecycle_events` + `Pr402Db::apply_escrow_lifecycle_step` (`migrations/003_escrow_lifecycle_events.sql`).
- [x] Ops CLI: `cargo run --bin record_escrow_lifecycle` (`src/bin/record_escrow_lifecycle.rs`).

### Post-outcome automation

- [x] `GET /api/v1/facilitator/sla-escrow-settle-cron` — DB-indexed Release/Refund ([CRON_OPERATIONS.md](./CRON_OPERATIONS.md)).
- [x] `POST /api/v1/facilitator/build-sla-escrow-settle-tx` — manual settle from `paymentUidHex` (chain-only; `src/bin/handlers/build_sla_escrow_settle_tx.rs`).
- [x] `GET /api/v1/facilitator/sla-escrow-close-cron` and `POST /api/v1/facilitator/sla-escrow-close`.
- [x] Standalone `settlement-keeper` binary with `SETTLEMENT_KEEPER_SLA_ESCROW_SOURCE=chain_scan` ([deploy/settlement-keeper/README.md](../deploy/settlement-keeper/README.md)).

### E2E (devnet)

- [x] `scripts/e2e/01_sla_escrow_facilitator_verify.sh` — buyer-paid fund + verify.
- [x] `scripts/e2e/03_sla_escrow_http_facilitator_fees.sh` — facilitator fee payer path.
- [x] `scripts/e2e/04_sla_escrow_post_fund_lifecycle.sh` — CLI lifecycle + DB audit.
- [x] `scripts/e2e/05_sla_escrow_full_cycle_devnet.sh` — chained full cycle.

---

## 3. Not shipped (HTTP boundary gap)

These routes are **not** registered in `src/bin/facilitator.rs`:

- `POST /api/v1/escrow/submit-delivery`
- `POST /api/v1/escrow/confirm-oracle`
- `POST /api/v1/escrow/release-payment`
- `POST /api/v1/escrow/refund-payment`

Integrators use **`sla-escrow` CLI**, custom tx builders (`sla_escrow_api` / `EscrowSdk`), or oracle binaries from the [`oracles`](https://github.com/miraland-labs/oracles) workspace.

---

## 4. Phased roadmap (remaining work)

### Phase A — Post-fund HTTP builders (SDK-first)

- [ ] `build-submit-delivery-tx`, `build-confirm-oracle-tx`, etc. — same pattern as fund builder (unsigned `VersionedTransaction` + notes).
- [ ] OpenAPI + `capabilities` entries for each route.
- [ ] Fee-payer matrix per instruction (do not blindly reuse slot-0 `sign()`).

### Phase B — Submit / relay endpoints

- [ ] Relay mode: client posts fully signed base64 tx; server validates + `send_transaction`.
- [ ] Facilitator co-sign only where message fee payer is facilitator.

### Phase C — Auth model

- [ ] Document who may call each step (seller, oracle, permissionless post-outcome).
- [ ] Optional API keys vs signature-only auth.

### Phase D — Observability

- [ ] Reconciliation job: `verify_ok` + `settle_ok IS NULL` + on-chain Funded → alert/backfill ([OPS_RECOVERY_PLAYBOOK.md](./OPS_RECOVERY_PLAYBOOK.md) §10).
- [ ] Optional webhooks for RP dashboards.

---

## 5. Architecture note

**Current:** monolith facilitator (verify/settle/cron/build on one deploy) — **recommended** until blast radius requires split.

**Future option:** thin `escrow-gateway` service for post-fund steps only.

---

## 6. Success criteria (when “done”)

- Integrator drives **fund → delivery → oracle → release** with **HTTP + one SDK** without `sla-escrow` binary on `PATH`.
- DB reflects lifecycle per **`payment_attempt`** traceably.
- E2E on devnet proves the path against preview/mainnet deployment.

---

*Planning document — implementation should land in separate PRs per phase.*
