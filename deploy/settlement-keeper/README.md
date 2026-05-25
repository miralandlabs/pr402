# Standalone Settlement Keeper deployment

Run the keeper as a separate always-on process when you want post-settlement
automation decoupled from the Vercel facilitator cron surface.

## Build

```bash
cargo build --release --bin settlement-keeper
```

## Required environment

| Variable | Purpose |
|---|---|
| `SOLANA_RPC_URL` | Cluster RPC |
| `FACILITATOR_KEYPAIR_BASE58` | Fee-payer for sweep / settle / close txs |
| `UNIVERSALSETTLE_PROGRAM_ID` | Exact rail (optional if only sla-escrow) |
| `SLA_ESCROW_PROGRAM_ID` | Escrow rail (optional if only exact sweep) |

## Optional environment

| Variable | Default | Purpose |
|---|---|---|
| `DATABASE_URL` | — | Read-only pr402 index for vault sweep + settle cooldown audit |
| `SETTLEMENT_KEEPER_INTERVAL_SEC` | `300` | Loop interval |
| `SETTLEMENT_KEEPER_DRY_RUN` | `false` | Log decisions without submitting txs |
| `SETTLEMENT_KEEPER_SLA_ESCROW_SOURCE` | `pr402_db` | `pr402_db` or `chain_scan` |
| `SETTLEMENT_KEEPER_VAULT_SWEEP_SOURCE` | `pr402_db` | `pr402_db` (chain scan not yet supported for sweep) |

## Example (Fly.io / Railway / VM)

```bash
export SETTLEMENT_KEEPER_SLA_ESCROW_SOURCE=chain_scan
export SETTLEMENT_KEEPER_INTERVAL_SEC=300
./target/release/settlement-keeper
```

## Docker

```bash
docker build -f deploy/settlement-keeper/Dockerfile -t settlement-keeper .
docker run --env-file .env.settlement-keeper.example settlement-keeper
```

Copy `.env.settlement-keeper.example` from the pr402 repo root and fill in secrets.
