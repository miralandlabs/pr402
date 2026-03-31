# pr402 devnet end-to-end (policy + full x402 verify/settle)

Scripts drive **Solana devnet** from your machine, hit a deployed **pr402** facilitator (default [preview.pr402.signer-payer.me](https://preview.pr402.signer-payer.me)), and optionally print **`payment_attempts` / `escrow_details`** via `DATABASE_URL` from `pr402/.env`.

## Test buyer wallet (fund this ATA)

Default **buyer** is whoever owns **`E2E_BUYER_KEYPAIR`** (default `~/.config/solana/id.json`). On this machine that resolves to:

```bash
solana address -k "${E2E_BUYER_KEYPAIR:-$HOME/.config/solana/id.json}"
```

Airdrop / mint **devnet USDC** for **`E2E_USDC_MINT`** (Circle devnet USDC in `common.sh`) to that wallet’s **USDC ATA** so both scenarios can pass **`/settle`** / `fund-payment`.

## Scenarios (devnet amounts)

| Scenario | Default amount | Rail | Script | x402 facilitator steps |
|----------|----------------|------|--------|-------------------------|
| **A** | **0.05 USDC** (`E2E_SCENARIO_A_AMOUNT_RAW=50000`) | UniversalSettle (`exact`) | `02_exact_facilitator_verify.sh` | `build-exact-payment-tx` → payer signs → **`/verify`** → **`/settle`** |
| **B2** | **1 USDC** | SLA-Escrow (facilitator pays Solana fees) | `03_sla_escrow_http_facilitator_fees.sh` | `build-sla-escrow-payment-tx` → buyer partial sign → **`/verify`** → **`/settle`** |
| **B1** | **1 USDC** | SLA-Escrow (buyer pays fees; CLI) | `01_sla_escrow_facilitator_verify.sh` | on-chain `fund-payment` → **`/verify`** → **`/settle`** |

**Production reference** (not enforced by these small defaults): `USDC_POLICY_THRESHOLD_WHOLE` (default 10) — below → prefer `exact`, at or above → prefer `sla-escrow`. Override amounts with env vars anytime.

x402 v2 expects a facilitator to support **verification** and **settlement** ([facilitator overview](https://docs.x402.org/core-concepts/facilitator)): **`/verify`** validates the payload; **`/settle`** completes (or acknowledges) on-chain execution. These scripts exercise **both** endpoints per scenario.

## SLA-Escrow “multiple steps” after x402

`delivery_signature`, `resolution_signature`, etc. in `escrow_details` cover **post-funding** on-chain steps (submit delivery, oracle confirmation, release/refund). Those are **not** the same as `/verify` and `/settle`; they are handled by the **sla-escrow program + CLI** after the buyer’s fund transaction. Run them separately when you need a full institutional lifecycle (see `sla-escrow` CLI: `submit-delivery`, `confirm-oracle`, `release-payment`, …).

## Prerequisites

- `solana`, `curl`, `jq`, `python3`, `cargo`
- Built **`sla-escrow-cli`**: `sla-escrow/target/release/sla-escrow` (admin features for open-escrow)
- Keypairs (defaults in `common.sh`): buyer funded with **SOL + USDC**; seller used as `payTo`; **UniversalSettle vault** for seller if `exact` is enabled on the deployment
- `RPC_URL` or `SOLANA_RPC_URL` — **use a reliable devnet RPC (e.g. Helius)** for `fund-payment` with `--amount-type human` (mint decimals) and for consistent simulation; plain `api.devnet.solana.com` often flakes.
- **USDC liquidity (green runs):**
  - **Scenario A:** payer’s USDC ATA must cover **`E2E_SCENARIO_A_AMOUNT_RAW`** (default **0.05 USDC**). `/settle` submits the transfer; “insufficient funds” means top up that ATA.
  - **Scenario B2:** same USDC as B1; **facilitator** pays SOL for the fund tx (buyer still needs SOL for rent/ops if any).
  - **Scenario B1:** payer must hold **`E2E_SCENARIO_B_AMOUNT_HUMAN`** USDC (default **1**) for `fund-payment`, plus **SOL** for buyer-paid fees.
  - Running **`run_all_devnet.sh`** (B2 + B1 + A): keep **at least ~2 USDC** (+ fees) on the buyer **ATA** for headroom.

## Commands

```bash
cd pr402/scripts/e2e
export RPC_URL="https://devnet.helius-rpc.com/?api-key=YOUR_KEY"
export FACILITATOR_URL="https://preview.pr402.signer-payer.me"
chmod +x *.sh

./02_exact_facilitator_verify.sh              # Scenario A (small exact amount)
./03_sla_escrow_http_facilitator_fees.sh      # Scenario B2 (SLA, facilitator fees)
./01_sla_escrow_facilitator_verify.sh         # Scenario B1 (SLA, CLI buyer-paid)
# or
./run_all_devnet.sh                           # B2 → B1 → A (see SKIP_* flags in script)
```

**Facilitator build:** B1 (**buyer-paid**) `/settle` expects a **fully signed** fund tx (often already on-chain). B2 (**facilitator-paid**) matches **A**: partial buyer sign, facilitator completes at `/settle`. See `docs/sla_escrow_fee_payer_and_settle.md`.

## Rust helpers

- `cargo run --example e2e_sign_exact_tx -- <payer.json> <recentBlockhash>`  
  Reads **unsigned** base64 tx on stdin; prints **signed** base64 for `/verify` and `/settle`.
- `cargo run --example e2e_sign_sla_escrow_tx -- <buyer.json> <recentBlockhash>`  
  Same for **`build-sla-escrow-payment-tx`** (facilitator fee payer: fills **buyer** signer slots only).

## Bincode

SLA-Escrow audit blobs use **`VersionedTransaction`** bincode compatible with `solana-transaction` 3.x (`decode_versioned_transaction_from_bincode` in pr402).
