# pr402 devnet end-to-end (HTTP + real transactions)

These scripts drive **Solana devnet** from your machine, hit a deployed **pr402** facilitator (default [preview.pr402.signer-payer.me](https://preview.pr402.signer-payer.me)), and optionally print **`payment_attempts` / `escrow_details`** rows via `DATABASE_URL` from `pr402/.env`.

## What runs

| Script | On-chain | Facilitator |
|--------|----------|-------------|
| `01_sla_escrow_facilitator_verify.sh` | `sla-escrow/scripts/open-escrow.sh` + `fund-payment.sh` (prints `AGENTIC_AUDIT_TX_B64`) | `POST /verify` (`sla-escrow`) |
| `02_exact_facilitator_verify.sh` | None extra (uses already-funded payer ATA) | `POST build-exact-payment-tx` → sign via `cargo run --example e2e_sign_exact_tx` → `POST /verify` (`exact`) |
| `run_all_devnet.sh` | Both | Both |

## Prerequisites

- `solana`, `curl`, `jq`, `python3`, `cargo`
- Built CLIs: `sla-escrow/target/release/sla-escrow`, `universalsettle/target/release/universalsettle` (exact path uses facilitator config only; vault must exist for your payee if required by deployment)
- Keypairs (defaults):
  - `E2E_BUYER_KEYPAIR` → `~/.config/solana/id.json`
  - `E2E_SELLER_KEYPAIR` → `~/.config/solana/test-id.json`
  - Buyer needs **SOL** + **USDC** on devnet for SLA fund; exact path needs payer **ATA** balance or will fail at verify simulation
- `RPC_URL` or `SOLANA_RPC_URL` (Helius devnet URL recommended)

## Commands

```bash
cd pr402/scripts/e2e
export RPC_URL="https://devnet.helius-rpc.com/?api-key=YOUR_KEY"
export FACILITATOR_URL="https://preview.pr402.signer-payer.me"
chmod +x *.sh
./01_sla_escrow_facilitator_verify.sh
# or
./run_all_devnet.sh
```

**Note:** `fund-payment.sh` embeds a default `ESCROW_CLI` path in some setups; this repo’s `01_*` script calls `sla-escrow/scripts/fund-payment.sh` with explicit `--rpc` / `--keypair`, which overrides RPC for the inner CLI. If `fund-payment` complains about a missing local `ESCROW_CLI` path, edit the `ESCROW_CLI=` line near the top of `sla-escrow/scripts/fund-payment.sh` or export a symlink to `target/release/sla-escrow`.

## Rust helper

- `cargo run --example e2e_sign_exact_tx -- <payer.json> <recentBlockhash>`  
  Reads **unsigned** base64 tx on stdin; prints **signed** base64 for `/verify`.

## Facilitator change (non-program)

Legacy `Transaction` bincode from `sla-escrow` CLI is accepted by `POST /verify` via `util::decode_versioned_transaction_from_bincode` in the pr402 crate.
