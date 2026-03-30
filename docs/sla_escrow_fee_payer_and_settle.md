# SLA-Escrow, fee payer, and x402 `/settle` (technical note)

This document records **implementation findings** about who pays Solana transaction fees (the **fee payer**), how that interacts with **`SolanaChainProvider::sign`**, and how that differs between **`v2:solana:exact`** (UniversalSettle) and **SLA-Escrow FundPayment** flows. It is intended for **future alignment** with x402’s *spirit* (facilitator-sponsored network fees where possible) and for **maintainers** changing CLI or facilitator settle logic.

## Solana prerequisite: “gas” follows the fee payer

On Solana there is no separate “gas payer” flag. The **fee payer** is the first account in the transaction message and **must sign** the message; SOL for fees is debited from that account. Any design where “only the buyer signs the payment but the facilitator pays gas” must be expressed as **concrete account ordering and signer positions** in the compiled message—not only as product intent.

## Current behavior in this repository

### `exact` (UniversalSettle) — facilitator is fee payer

`build-exact-payment-tx` builds a legacy/Versioned shell where **`Some(&fee_addr)` is the facilitator** (`provider.fee_payer()`). The buyer (`payer`) signs as token authority for `TransferChecked` (and any ATA-create idempotent ix), but **signature index 0 is reserved for the facilitator** fee payer at **`/settle`**.

See: `pr402/src/exact_payment_build.rs` (module docs + `Message::new_with_blockhash(..., Some(&fee_addr), ...)`), and `pr402/src/chain/solana.rs` `sign()` which sets `tx.signatures[0]`.

### SLA-Escrow `fund-payment` (CLI today) — buyer is fee payer

The `sla-escrow` CLI builds the fund transaction with **`new_with_payer(..., Some(&signer.pubkey()))`** where `signer` is the buyer CLI keypair. So **today’s FundPayment transactions are buyer fee payer + buyer signs slot 0**.

This is **not** prescribed by x402; it is a **consequence of the current CLI construction**. The x402 **spirit** (“buyer signs payment; facilitator pays gas”) would require a **different message layout**: facilitator as fee payer (first signer), buyer as required signer on the escrow instruction accounts, similar in *shape* to the `exact` shell—with whatever extra signers FundPayment needs.

### Why `settle_transaction` was “correct for exact, wrong for FundPayment”

`SolanaChainProvider::sign` **always** assigns:

```text
tx.signatures[0] = facilitator_keypair.sign_message(message.serialize())
```

That matches **`exact`** shells where index 0 **is** the facilitator fee payer.

It **does not** match **buyer-fee-payer FundPayment** shells where index 0 **must** remain the **buyer’s** signature. Overwriting slot 0 **invalidates** the transaction (wrong key signed the fee-payer slot).

**Mitigation in code:** `v2_solana_escrow` settle uses `settle_sla_escrow_fund_payment` for fully buyer-signed fund txs: confirm the existing primary signature on-chain, submit unchanged if still pending, or treat “already processed” as success when the fund tx was already landed before `/verify`.

Location: `pr402/src/scheme/v2_solana_escrow/mod.rs` (`settle_sla_escrow_fund_payment`).

## Future work (“later fix”) — optional product goals

1. **Align FundPayment with facilitator-paid fees (if desired)**  
   - Change `sla-escrow` CLI (or add a “facilitator fee payer” build mode) so the **fee payer account** is the facilitator pubkey, and the **buyer** only signs the instruction(s) that require buyer authority—mirroring the documented pattern in `exact_payment_build.rs`.  
   - Then **`/settle`** can reuse the same **`sign` + send** pattern as `exact` without special casing, *provided* the message’s signer order matches `sign()`’s assumption.

2. **Generalize `SolanaChainProvider::sign` (if multi-layout)**  
   - Today it assumes fee payer is always at index 0. A more robust approach is to sign **by fee payer index** from the message header (or explicit metadata) so different layouts don’t silently break.

3. **`escrow_details.fund_signature` vs settlement response**  
   - Audit whether DB writes should store **on-chain fund tx signature** vs facilitator **settlement** acknowledgment consistently; align `persist_escrow_audit_after_settle` and HTTP response `transaction` field with product expectations.

4. **Extended SLA lifecycle persistence**  
   - Columns such as `delivery_signature`, `resolution_signature`, `delivery_hash` are not populated by verify/settle alone; wire them when `submit-delivery`, `confirm-oracle`, `release-payment`, etc. are integrated with the facilitator or a backend indexer.

## What’s next (suggested order)

1. **Decide product stance:** Should SLA-Escrow FundPayment on the **public** agent path always use **facilitator-paid** fees (matching `exact`), or is **buyer-paid** fees acceptable for v1?  
2. **If facilitator-paid:** implement CLI + on-chain account metas so facilitator is fee payer; add E2E that asserts fee payer pubkey in the serialized message; simplify escrow settle to the shared path where safe.  
3. **If buyer-paid remains:** keep `settle_sla_escrow_fund_payment` idempotent behavior; document in user-facing integration guides that buyers need SOL for fees on fund txs.  
4. **Run** `pr402/scripts/e2e/run_all_devnet.sh` against a deployment that includes the latest escrow settle logic; confirm `payment_attempts.settle_ok` and `escrow_details` for scenario B.  
5. **Telemetry / DB hardening:** close remaining items in `pr402/docs/facilitator_audit_status.md` (e.g. `server_log` target, stable DB verification) as needed for production.

---

*Last updated: records fee-payer / `sign()` slot-0 analysis and idempotent SLA settle rationale.*
