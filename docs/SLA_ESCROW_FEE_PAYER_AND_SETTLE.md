# SLA-Escrow, fee payer, and x402 `/settle` (technical note)

Maintainer reference: who pays Solana **transaction fees** (the message **fee payer**), how that interacts with **`SolanaChainProvider::sign`**, and how **`/verify`** and **`/settle`** branch between **`exact`** and **sla-escrow** layouts.

**Related:** [SLA_ESCROW_FULLCYCLE_ROADMAP.md](./SLA_ESCROW_FULLCYCLE_ROADMAP.md) · [OPS_RECOVERY_PLAYBOOK.md](./OPS_RECOVERY_PLAYBOOK.md)

---

## Solana prerequisite

The **fee payer** is the first account in the transaction message and **must sign** slot 0. SOL for fees is debited from that account. “Facilitator pays gas” must be expressed as account ordering and signer slots — not product intent alone.

---

## Current behavior (verified against source)

### `exact` (UniversalSettle)

- **`build-exact-payment-tx`:** facilitator is message fee payer (`provider.fee_payer()`).
- **`/settle`:** `settle_transaction` → `SolanaChainProvider::sign` overwrites **signature slot 0** with the facilitator key.

**Code:** `src/exact_payment_build.rs`, `src/chain/solana.rs` (`sign`), `src/scheme/v2_solana_exact/`.

### SLA-Escrow `FundPayment` — default HTTP builder: **buyer** fee payer

- **`POST /build-sla-escrow-payment-tx`:** JSON field **`facilitatorPaysTransactionFees`** maps to `BuildSlaEscrowPaymentTxRequest.facilitator_pays_transaction_fees`, which defaults to **`false`** (`#[serde(default)]`).
- **Default shell:** buyer is sole fee payer / sole required signer (matches **`sla-escrow` CLI** buyer-paid layout).
- **`/settle`:** uses **`settle_sla_escrow_fund_payment`** — confirm or submit the **fully buyer-signed** tx; does **not** overwrite slot 0 with the facilitator key.

**Code:** `src/sla_escrow_payment_build.rs`, `src/scheme/v2_solana_escrow/mod.rs` (`settle`, `settle_sla_escrow_fund_payment`).

### SLA-Escrow — optional facilitator fee payer (opt-in)

- Request body: **`facilitatorPaysTransactionFees: true`**.
- **HTTP gate:** returns **400** unless deployment sets **`PR402_SLA_ESCROW_ALLOW_FACILITATOR_FEE_SPONSORSHIP=1`** (or `true` / `yes`). Checked in `src/bin/handlers/build.rs` against `ChainProvider::sla_escrow_allow_facilitator_fee_sponsorship` (`src/config.rs`).
- **Shell:** facilitator at message index 0, buyer as second signer on `FundPayment` (same pattern as `exact`).
- **`/verify`:** if first static account is facilitator → `sign(provider)` on slot 0 before simulate.
- **`/settle`:** **`settle_transaction`** (facilitator signs slot 0 + broadcast).

### Branch rule in `settle`

```text
sponsor_is_facilitator =
  transaction.message.static_account_keys()[0] == facilitator_pubkey

if sponsor_is_facilitator:
    settle_transaction(...)              // exact + facilitator-sponsored SLA
else:
    settle_sla_escrow_fund_payment(...)  // buyer-paid SLA
```

**Verify simulation:** facilitator-sponsored → `sign(provider)` before simulate; buyer-paid → **no** facilitator `sign()` on the client tx (preserve buyer signature on slot 0).

**Locations:** `src/scheme/v2_solana_escrow/mod.rs` (`verify_transfer`, `settle`).

---

## FundPayment.seller (payout wallet)

HTTP builder requires **`accepted.extra.beneficiary`** or **`accepted.extra.merchantWallet`** — encoded as on-chain **`payment.seller`**. Must **not** be the escrow PDA.

**Code:** `resolve_fund_payment_seller_pk` in `src/sla_escrow_payment_build.rs`; verify checks in `src/scheme/v2_solana_escrow/mod.rs`.

---

## Invariants (regression checklist)

| Invariant | Enforcement |
|-----------|-------------|
| Facilitator pubkey must **not** appear in FundPayment **instruction account metas** | `verify_transfer` → `FeePayerIncludedInInstructionAccounts` |
| **`settle_transaction` only** when message fee payer == facilitator | `settle` → `sponsor_is_facilitator` branch |
| Buyer-paid **`/settle`** requires fully signed tx before submit | `settle_sla_escrow_fund_payment` |
| Facilitator-sponsored SLA: ≥ 2 signers when buyer ≠ facilitator | `verify_transfer` |

---

## Parity matrix

| Aspect | `exact` | SLA buyer-paid (default HTTP) | SLA facilitator-paid (opt-in) |
|--------|---------|-------------------------------|-------------------------------|
| Default builder fee payer | Facilitator | Buyer | Facilitator (if env gate on) |
| `/settle` path | `settle_transaction` | `settle_sla_escrow_fund_payment` | `settle_transaction` |
| HTTP build flag | N/A | `facilitatorPaysTransactionFees: false` (default) | `true` + `PR402_SLA_ESCROW_ALLOW_FACILITATOR_FEE_SPONSORSHIP` |

---

## DB audit note (ops)

After **`/settle`** succeeds, `payment_attempts.settlement_signature` is written in `src/bin/handlers/settle.rs`. Escrow audit (`fund_signature`, `payment_uid_hex`, PDAs) runs next via `persist_escrow_audit_after_settle` in `src/scheme/v2_solana_escrow/mod.rs` — **best-effort** (`warn` on extract failure, does not fail the HTTP response). See [OPS_RECOVERY_PLAYBOOK.md](./OPS_RECOVERY_PLAYBOOK.md) when those diverge.

---

## E2E coverage (devnet)

| Script | Scenario |
|--------|----------|
| `scripts/e2e/03_sla_escrow_http_facilitator_fees.sh` | Facilitator fee payer (requires env gate on deployment) |
| `scripts/e2e/01_sla_escrow_facilitator_verify.sh` | Buyer-paid CLI fund → verify |
| `scripts/e2e/run_all_devnet.sh` | Orchestrates SLA + exact paths |

---

*Last updated: aligned with `BuildSlaEscrowPaymentTxRequest` default `facilitator_pays_transaction_fees: false` and HTTP sponsorship gate.*
