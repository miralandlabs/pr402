# pr402 facilitator — code review vs `universalsettle-api` v0.1.8 & `sla-escrow-api` v0.2.9

**Date:** 2026-04-23  
**Scope:** Align pr402 with published on-chain program APIs/SDKs; assess **logic correctness** and **usability** for naive sellers (resource providers) and naive buyer agents.  
**Constraint (original review):** Findings and recommendations only — **no code changes** in this pass.

**Follow-up (2026-04-23):** The items below were **implemented in code** where marked ✅.

| Finding | Status |
|--------|--------|
| `FundPayment.seller` must be merchant wallet | ✅ `sla_escrow_payment_build.rs` + verify in `v2_solana_escrow/mod.rs` |
| UniversalSettle vault ATA / Token-2022 | ✅ `ensure_vault_setup` in `chain/solana.rs` |
| SLA builder Token-2022 (plain mint) | ✅ `build_fund_payment_instruction` + payment build |
| `get_payment_address` token program | ✅ optional `spl_token_program` arg in `solana_universalsettle.rs` |
| SLA build response blockhash hint | ✅ `recentBlockhashExpiresAt` on `BuildSlaEscrowPaymentTxResponse` |

**Breaking change for integrators:** SLA escrow **verify** now requires `paymentRequirements.extra.merchantWallet` or `beneficiary`, and the built tx must encode the same pubkey as `FundPayment.seller`. `accepted.extra` on the build request must include one of these fields.

---

## Executive summary

pr402 largely **re-implements instruction layouts by hand** (`chain/solana_universalsettle.rs`, `chain/solana_sla_escrow.rs`) instead of calling `universalsettle_api::sdk` / `sla_escrow_api::sdk` directly. That is defensible for Solana **3.x** dependency isolation, but it raises the bar for **keeping discriminators, account order, and `repr(C)` layouts in sync** with the crates you publish.

**Already well aligned (high confidence):**

- UniversalSettle **`Sweep`** / **`CreateVault`** discriminators and account metas match the current `universalsettle-api` instruction definitions (including **writable vault** in sweep, config readonly, SPL account order ending with mint + token program).
- On-chain **Config** and **SplitVault** POD layouts in `solana_universalsettle.rs` match `universalsettle-api` (authority, fee_destination, `use_fee_shard`, `shard_count`, single `is_provisioned`, etc.).
- Cron **sweep** loads live config from chain and uses `sweep_fee_destinations(...)` consistent with on-chain fee routing when sharding is enabled.

**Must-fix gaps (logic / safety):**

- **SLA-Escrow `FundPayment` seller field:** the HTTP builder sets `seller = escrow_pda` when calling `build_fund_payment_instruction`. On-chain, `payment.seller` is the **payout wallet** checked on `ReleasePayment` / `RefundPayment` (`seller_info.key` must equal `payment.seller`). Using the **escrow PDA** as `seller` is **not equivalent** to the merchant wallet and will break or dangerously mis-route settlement unless every downstream path compensates (it does not). **Recommendation:** derive `seller` from `accepted.extra.merchantWallet` or `accepted.extra.beneficiary` (with documented precedence), never from `escrow_pda`.

**Should-fix gaps (alignment / future bugs):**

- **Token-2022 split brain:** `exact_payment_build.rs` supports Token-2022 mints for buyer transfers, but `ensure_vault_setup` always creates/finds the vault ATA with **legacy `TOKEN_PROGRAM_ID`**. For a Token-2022 mint, the vault’s ATA is under **Token-2022**, not legacy Token — provisioning and sweep can target the **wrong ATA**. **Recommendation:** resolve mint owner → token program once; thread it through `ensure_vault_setup`, ATA creation, and any helper that uses `get_payment_address` (which today hard-codes legacy Token for SPL).

- **SLA-Escrow builder rejects Token-2022** while on-chain `sla-escrow` supports **plain** Token-2022 mints (extensions blocked). **Recommendation:** either document “classic Token only” as a deliberate facilitator policy, or extend the builder to mirror `EscrowSdk` / program layout with `associated_token_account_with_program`.

- **Optional:** replace hand-rolled instruction bytes with thin wrappers around `universalsettle_api::sdk::*` / `sla_escrow_api::sdk::EscrowSdk::*` where dependency graphs allow, **or** add a small `#[test]` module that asserts byte-for-byte equality against the published SDK for a golden vector per instruction (discriminator + accounts + data).

---

## 1. API / SDK alignment checklist

### 1.1 UniversalSettle

| Area | pr402 location | Assessment |
|------|----------------|------------|
| `CreateVault` accounts | `build_create_vault_instruction` | Matches SDK: payer, vault, vault_sol_storage, system program. |
| `Sweep` accounts (SOL / SPL) | `build_sweep_instruction` | Matches SDK order; SPL passes mint + token program last; vault is writable. |
| `Sweep` data layout | `SweepData::to_bytes` | 32 + 8 + 1 + 7 padding — matches `instruction::Sweep` pod layout. |
| Fee sharding | `sweep_fee_destinations` + cron `execute_sweep` | Uses on-chain `use_fee_shard` + `shard_count` — correct pattern vs env-only config. |
| Config POD | `chain::solana_universalsettle::Config` | Matches `universalsettle_api::state::Config` field order and sizes. |
| SplitVault POD | `SplitVault` in same module | Matches `split_vault.rs` (including `is_provisioned` semantics). |

**Note:** pr402 does **not** need the removed redundant `mint_account` argument from an older SDK discussion — its manual builder already uses `token_mint` once for SPL.

### 1.2 SLA-Escrow

| Area | pr402 location | Assessment |
|------|----------------|------------|
| `FundPayment` discriminator + payload | `solana_sla_escrow.rs` | Layout matches API `FundPayment` wire format. |
| Account metas (SPL path) | `build_fund_payment_instruction` | Uses legacy Token program only — **misaligned** with on-chain Token-2022 support. |
| **`seller` argument** | `sla_escrow_payment_build.rs` | **Incorrect** — see executive summary. |

### 1.3 Event indexing (off-chain)

On-chain programs now emit structured events via **`log()`** (`sol_log_data`), not `set_return_data`. pr402 indexers or auditors should subscribe to **transaction logs**, not return data. No pr402 code change required for this review item — document for anyone parsing facilitator-side traces.

---

## 2. Logic correctness — deeper findings

### 2.1 Critical: `FundPayment.seller` in SLA-Escrow HTTP builder

**File:** `src/sla_escrow_payment_build.rs`  
**Issue:** `let seller = escrow_pda;` passed into `build_fund_payment_instruction`.  
**On-chain contract:** `payment.seller` is set from `args.seller` and later compared to the **seller wallet** account on release/refund.  
**Impact:** Merchant payout destination is wrong; release/refund account preparation for naive integrators will not match escrow semantics.  
**Recommendation:** Require `accepted.extra.merchantWallet` (or explicit `beneficiary`) and pass that pubkey as `seller`; validate it is a plausible wallet (not a program-owned escrow account).

### 2.2 High: Token-2022 ATA domain for UniversalSettle provisioning

**Files:** `src/chain/solana.rs` (`ensure_vault_setup`), `src/chain/solana_universalsettle.rs` (`get_payment_address`)  
**Issue:** Vault ATA derivation / creation uses **legacy** token program even when the payment mint is Token-2022-owned.  
**Impact:** Funds may be sent to an ATA the vault never uses; sweeps may read zero while user “paid” the wrong account.  
**Recommendation:** Unify token-program detection (same as `exact_payment_build.rs` / cron sweep path) for provisioning + any “expected pay address” helpers.

### 2.3 Medium: SLA-Escrow builder policy vs program capability

**File:** `src/sla_escrow_payment_build.rs` — rejects Token-2022 mints.  
**Assessment:** Safe **product** choice if documented; **incorrect** if marketed as “full parity with sla-escrow mainnet”.  
**Recommendation:** State explicitly in `public/agent-integration.md` / OpenAPI descriptions: “USDC/USDT classic Token only” unless builder is extended.

### 2.4 Medium: Shard / config drift visibility for integrators

Cron sweep correctly prefers **on-chain** `fee_destination` and shard flags. Env/config in the facilitator can drift; a warning is logged when `fee_destination` differs.  
**Recommendation:** Surface `use_fee_shard`, `shard_count`, and treasury in **`/supported`** extra for UniversalSettle so sellers debugging “where did fees go?” do not need RPC introspection.

### 2.5 Low: `get_payment_address` helper

**File:** `chain/solana_universalsettle.rs`  
For SPL, uses `spl_token::id()` only. Misleading if called for Token-2022 rails.  
**Recommendation:** Deprecate or add `token_program` parameter.

### 2.6 Low: Blockhash lifetime

**Files:** `exact_payment_build.rs`, `sla_escrow_payment_build.rs`  
Exact path exposes `recentBlockhashExpiresAt` (rough +60s). SLA builder does not expose the same field.  
**Recommendation:** Harmonize response shape for agent clients.

---

## 3. Usability — naive seller & naive buyer agent

### 3.1 Strengths

- **Two-phase flow** (build → sign → verify/settle) is appropriate for x402 v2.
- **Notes** arrays in build responses give human guidance (fee payer, signature order).
- **Exact rail:** `payerSignatureIndex` and fee-payer semantics are spelled out — good for agents.
- **SLA rail:** Oracle allowlist validation against `accepted.extra.oracleAuthorities` reduces misconfiguration.
- **Cron sweep** with dry-run, thresholds, and DB-backed cooldowns is operationally thoughtful.

### 3.2 Pain points & recommendations

| Audience | Issue | Recommendation |
|----------|--------|----------------|
| Buyer agent | Multiple JSON shapes (`accepted`, `paymentPayload`, `paymentRequirements`) | Publish a single **minimal** example per rail in OpenAPI + link to frozen JSON schema. |
| Buyer agent | Blockhash expiry handling inconsistent across builders | Always return `recentBlockhashExpiresAt` (or omit from both with explicit doc). |
| Seller | UniversalSettle vs SLA mental model | Short diagram in docs: “exact + vault = streaming payout; escrow = dispute/oracle path”. |
| Seller | `merchantWallet` / `beneficiary` semantics | Document required fields per rail; **especially** once `FundPayment.seller` is fixed. |
| Operator | Token-2022 “sometimes works” (exact but not escrow / wrong ATA) | Until fixed, **fail closed** or banner “Token-2022 experimental” on build endpoints. |

### 3.3 Observability

- Structured **tracing** exists in `facilitator` bin — good.
- For merchants, expose correlation IDs consistently in error JSON (already partially present).

---

## 4. Recommended follow-up work (prioritized)

1. **P0 — Fix SLA-Escrow fund builder `seller`** (merchant wallet from `extra`, never escrow PDA). Add a regression test that decodes instruction data and asserts `seller` equals configured merchant.
2. **P0 — Fix UniversalSettle provisioning ATA program id** for Token-2022 mints (thread token program from mint account owner).
3. **P1 — Decide Token-2022 policy for SLA-Escrow** (implement or document exclusion).
4. **P1 — Harmonize build response metadata** (blockhash expiry, optional shard fields in `/supported`).
5. **P2 — Golden-vector tests** vs published SDK encodings to prevent silent drift on crate bumps.

---

## 5. Conclusion

pr402’s **UniversalSettle** integration is structurally aligned with **v0.1.8**-era layouts and already respects **fee sharding** when sweeping. The largest **logic** risk found is **`FundPayment.seller` being set to the escrow PDA** in `sla_escrow_payment_build.rs`, which does not match on-chain payout semantics. The largest **alignment** risk is **Token-2022**: supported on the exact rail but not consistently propagated into vault provisioning / SLA builder.

No facilitator code was modified in this review; the above items are intended as a merge-ready checklist for a follow-up PR.
