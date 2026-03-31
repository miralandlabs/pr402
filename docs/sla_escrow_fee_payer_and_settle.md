# SLA-Escrow, fee payer, and x402 `/settle` (technical note)

This document records **implementation findings** about who pays Solana transaction fees (the **fee payer**), how that interacts with **`SolanaChainProvider::sign`**, and how that differs—or now **aligns**—between **`v2:solana:exact`** (UniversalSettle) and **SLA-Escrow FundPayment** flows. It is intended for **maintainers** changing CLI, HTTP builders, or facilitator settle logic.

## Solana prerequisite: “gas” follows the fee payer

On Solana there is no separate “gas payer” flag. The **fee payer** is the first account in the transaction message and **must sign** the message; SOL for fees is debited from that account. Any design where “only the buyer signs the payment but the facilitator pays gas” must be expressed as **concrete account ordering and signer positions** in the compiled message—not only as product intent.

## Current behavior in this repository

### `exact` (UniversalSettle) — facilitator is fee payer

`build-exact-payment-tx` builds a legacy/Versioned shell where **`Some(&fee_addr)` is the facilitator** (`provider.fee_payer()`). The buyer (`payer`) signs as token authority for `TransferChecked` (and any ATA-create idempotent ix), but **signature index 0 is reserved for the facilitator** fee payer at **`/settle`**.

See: `pr402/src/exact_payment_build.rs`, and `pr402/src/chain/solana.rs` `sign()` which sets `tx.signatures[0]`.

### SLA-Escrow `FundPayment` — facilitator-paid by default (Phase 5)

**`POST /api/v1/facilitator/build-sla-escrow-payment-tx`** (default `buyerPaysTransactionFees: false`) builds the same *shape*: **facilitator as message fee payer**, buyer as **second** required signer on `FundPayment` (and ATA-create funding accounts still use the **buyer** so the facilitator is not referenced inside instruction metas—required by `verify_transfer`).

- **`/verify`:** If the message’s first account is the facilitator, simulation uses `TransactionInt::sign(provider)` on slot 0 (same as `exact`). Buyer must have partially signed their slot(s) first.
- **`/settle`:** Uses the shared **`settle_transaction`** path (sign fee payer + `send_and_confirm`), not the legacy idempotent buyer-paid helper.

### Legacy: buyer fee payer (CLI and opt-in HTTP field)

The `sla-escrow` CLI still builds with **`new_with_payer(..., Some(&buyer))`**. Integrators can match that by calling **`build-sla-escrow-payment-tx`** with **`buyerPaysTransactionFees: true`**.

For this layout, **`/settle`** continues to use **`settle_sla_escrow_fund_payment`**: confirm or submit the **fully buyer-signed** tx without overwriting slot 0 with the facilitator key.

### Why `settle_transaction` was “correct for exact, wrong for buyer-paid FundPayment”

`SolanaChainProvider::sign` **always** assigns:

```text
tx.signatures[0] = facilitator_keypair.sign_message(message.serialize())
```

That matches **`exact`** shells where index 0 **is** the facilitator fee payer.

It **does not** match **buyer-fee-payer FundPayment** shells where index 0 **must** remain the **buyer’s** signature. Overwriting slot 0 **invalidates** the transaction for broadcast.

**Mitigation:** `v2_solana_escrow` settle branches on the message’s first static account: facilitator fee payer → **`settle_transaction`**; buyer fee payer → **`settle_sla_escrow_fund_payment`**.

**Verify simulation:** Facilitator-sponsored shells use `sign(provider)` before `simulate_transaction` (slot 0 = facilitator). Buyer-paid shells **do not** call `sign(provider)` so the client’s buyer signature on slot 0 is preserved for simulation (`sig_verify` remains off).

Locations: `pr402/src/scheme/v2_solana_escrow/mod.rs` (`verify_transfer`, `settle`).

## Product decisions (resolved defaults)

| Question | Suggestion implemented |
|----------|------------------------|
| Default for agent / HTTP path? | **Facilitator pays network fees** (x402-aligned with `exact`). |
| Keep CLI / old scripts? | **Yes** — buyer-paid via CLI; HTTP via `buyerPaysTransactionFees: true`. |
| Discovery? | `/supported` SLA-Escrow `extra.slaFundTxNetworkFeePayer` is **`"facilitator"`** (JSON field name). |

**Open follow-up:** `sla-escrow` CLI could gain a “print unsigned + facilitator fee payer” mode that does **not** auto-send (needs facilitator co-sign offline). Until then, use the facilitator **HTTP** builder.

## Remaining roadmap items

1. **`escrow_details.fund_signature` vs settlement response** — Audit whether DB should store **on-chain fund tx** vs facilitator **submission** signature consistently.
2. **Extended SLA lifecycle persistence** — `delivery_signature`, `resolution_signature`, etc.; see [sla_escrow_fullcycle_roadmap.md](./sla_escrow_fullcycle_roadmap.md).
3. **Generalize `SolanaChainProvider::sign`** — Optional: sign by explicit fee-payer index from the message header for exotic layouts.

## Self-review — fee payer architecture vs x402 v2

This section is a **maintainer checklist** after Phase 5 + dual E2E paths; it is not normative x402 text.

### x402 v2 facilitator contract (what we comply with)

- **Discovery:** `GET /supported` lists payment kinds (`scheme`, `network`, `extra`). SLA-Escrow `extra` includes oracle allow-list, program PDAs, and `slaFundTxNetworkFeePayer` so agents know the **default** HTTP-built shell.
- **Verify:** `POST /verify` accepts a v2 body with **`paymentPayload.accepted`** mirroring **`paymentRequirements`**, and a rail-specific **`payload`** (here: base64 bincode `VersionedTransaction`). pr402 validates instruction shape, amounts, oracle allow-list, and simulates with RPC (`sig_verify: false` as elsewhere in Solana facilitator practice).
- **Settle:** `POST /settle` uses the **same JSON shape** as verify for v2; optional **`correlationId`** / header merges DB rows. The response returns a **transaction** signature string for the submission path chosen by the rail.
- **Naming:** `build-exact-payment-tx` vs `build-sla-escrow-payment-tx` are explicit product endpoints—not a single ambiguous “build tx” route.

### Invariants (must stay true on every change)

| Invariant | Where enforced |
|-----------|----------------|
| Facilitator pubkey must **not** appear in **instruction account metas** for SLA `FundPayment` txs | `verify_transfer` loop (`FeePayerIncludedInInstructionAccounts`) |
| Message fee payer is `static_account_keys()[0]` and must sign slot 0 | Solana runtime; `SolanaChainProvider::sign` assumes slot 0 = facilitator when used |
| Facilitator-sponsored SLA: **≥ 2** signers when buyer ≠ facilitator | `verify_transfer` (`num_required_signatures`) |
| **`settle_transaction` (overwrites slot 0)** only for layouts whose fee payer is the facilitator | `settle`: branch on `verification.transaction…[0] == facilitator`; **never** call from `settle_sla_escrow_fund_payment` |
| Buyer-paid SLA: **`/settle` requires a fully signed tx** (buyer completed slot 0 before submit / idempotent confirm) | `settle_sla_escrow_fund_payment`: `!is_fully_signed` → error (avoids corrupting buyer sig with facilitator `sign()`) |
| Verify simulation: facilitator-sponsored → `sign(provider)`; buyer-paid → **no** facilitator `sign()` on the client tx | `verify_transfer` |

### Parity matrix (rails)

| Aspect | `exact` | SLA facilitator fees | SLA buyer-paid |
|--------|---------|----------------------|----------------|
| Default builder fee payer | Facilitator | Facilitator | Buyer (CLI / opt-in HTTP) |
| Buyer signature timing before `/verify` | Partial (non–fee-payer slots) | Partial | Full (CLI broadcasts; or slot 0 filled) |
| `/settle` submit path | `settle_transaction` | `settle_transaction` | `settle_sla_escrow_fund_payment` |
| `UniversalSettle` sweep after pay | Yes | N/A (extract fails; logged) | N/A |

### Residual risks / out of scope

- **Token-2022** and **native SOL** `FundPayment` in HTTP builders: still unsupported or CLI-only; verify assumptions match builders.
- **Same keypair** as buyer and facilitator: Solana may **dedupe** signers → `num_required_signatures` can be 1; facilitator branch still applies if message fee payer pubkey equals facilitator.
- **Preview / deploy drift:** E2E `03_sla_escrow_http_facilitator_fees.sh` requires a deployment that includes **both** `build-sla-escrow-payment-tx` and Phase 5 settle logic.

### E2E coverage

- **B2** — `03_sla_escrow_http_facilitator_fees.sh`: facilitator fees, HTTP build, partial buyer sign, verify/settle submission.
- **B1** — `01_sla_escrow_facilitator_verify.sh`: buyer-paid CLI fund before verify.
- **`run_all_devnet.sh`:** runs B2 then B1 then A; `SKIP_SLA_HTTP` / `SKIP_SLA_CLI` split SLA scenarios.

---

*Last updated: Phase 5 + buyer-paid settle guard + dual SLA E2E; self-review section for x402 v2 alignment.*
