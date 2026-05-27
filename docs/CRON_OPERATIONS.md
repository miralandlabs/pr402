# Cron operations (Vercel deployment)

pr402 ships scheduled cron jobs that drive on-chain housekeeping via the optional
**Settlement Keeper** role. Both primary crons are HTTP `GET` endpoints invoked by
Vercel's built-in cron scheduler (or an external fallback — see below).

**Stuck or inconsistent payments?** See **[OPS_RECOVERY_PLAYBOOK.md](./OPS_RECOVERY_PLAYBOOK.md)**
for manual recovery when DB rows and chain state diverge (verify-only rows, missing
`fund_signature`, split-brain settle, cron skips, etc.).

| Endpoint | Default schedule | Purpose |
|---|---|---|
| `GET /api/v1/facilitator/sweep-cron` | `*/5 * * * *` | **Vault sweeper** — UniversalSettle `Sweep` for merchant payouts |
| `GET /api/v1/facilitator/sla-escrow-settle-cron` | `*/5 * * * *` | **Escrow settler** — `ReleasePayment` / `RefundPayment` post-outcome |
| `GET /api/v1/facilitator/sla-escrow-close-cron` | (external only) | **Escrow closer** — `ClosePayment` rent reclamation after terminal state |

Both primary schedules and routes are declared in [`pr402/vercel.json`](../vercel.json).

## Phase 0 activation checklist

Before expecting crons to run in production, verify every item:

1. **Vercel plan** includes cron jobs; [`vercel.json`](../vercel.json) lists both paths.
2. **`CRON_SECRET`** is set in Vercel project env and the project was **redeployed** after setting it.
3. **`PR402_SWEEP_CRON_TOKEN`** and **`PR402_SLA_ESCROW_SETTLE_CRON_TOKEN`** match `CRON_SECRET`
   (or are set in the `parameters` table — DB wins over env).
4. **`DATABASE_URL`** is configured (handlers return 503 without it for DB-indexed crons).
5. Migration **`007_escrow_payment_uid.sql`** applied on existing DBs; fresh `init.sql` includes
   `payment_uid_hex` and seeds SLA-Escrow settle cron parameters.
6. **Dry-run smoke test** (no on-chain txs):

   ```bash
   PR402_BASE_URL=https://your-deployment CRON_SECRET=... \
     ./scripts/trigger-settlement-crons.sh dry-run
   ```

7. **Health check** — `GET /api/v1/facilitator/health` includes `settlementKeeper`:
   `vaultSweepCronConfigured`, `slaEscrowSettleCronConfigured`, `databaseConnected`.

8. **External fallback** — if Vercel crons are unavailable, enable
   [`.github/workflows/settlement-keeper-cron.yml`](../.github/workflows/settlement-keeper-cron.yml)
   with repository secrets `PR402_BASE_URL` and `CRON_SECRET`.

9. **Standalone worker** (optional) — deploy `cargo run --bin settlement-keeper` on an always-on
   host; see [`deploy/settlement-keeper/README.md`](../deploy/settlement-keeper/README.md).

## Authentication

Vercel's cron scheduler attaches `Authorization: Bearer <CRON_SECRET>` on
every cron-triggered request when `CRON_SECRET` is set in the project's
environment variables. Our handlers validate this header against
per-cron bearer tokens stored in the params table or env.

**Recommended setup**: one secret value, three references.

1. Generate a strong random secret (e.g., `openssl rand -hex 32`).
2. Set the same value in three places:

   | Variable | Where | Purpose |
   |---|---|---|
   | `CRON_SECRET` | Vercel project env | Vercel injects this as bearer on cron-triggered requests |
   | `PR402_SWEEP_CRON_TOKEN` | Vercel project env or params table | Validates bearer for `sweep-cron` |
   | `PR402_SLA_ESCROW_SETTLE_CRON_TOKEN` | Vercel project env or params table | Validates bearer for `sla-escrow-settle-cron` |

3. Redeploy the Vercel project for `CRON_SECRET` to take effect.

If you want distinct tokens per cron (separate rotation schedules), you
can set `PR402_SWEEP_CRON_TOKEN` and `PR402_SLA_ESCROW_SETTLE_CRON_TOKEN`
to different values — but Vercel's built-in cron only injects one
`CRON_SECRET`, so distinct values would require switching to an
external scheduler (Upstash QStash, GitHub Actions, Cloudflare Workers)
that can supply the per-cron bearer.

The recommended single-secret setup is the simplest configuration that
satisfies both crons. Rotation: pick a maintenance window, set a new
secret in all three places, redeploy.

## SLA-Escrow settlement cron — operator parameters

Beyond the bearer token, the SLA-Escrow settlement cron exposes four
tuning parameters. All have sensible defaults; override only when
operational data justifies it.

| Variable | Default | Purpose |
|---|---|---|
| `PR402_SLA_ESCROW_SETTLE_CRON_BATCH_LIMIT` | 15 | Max candidates fetched per cron run. Sized to fit Vercel's 60s function timeout with margin. |
| `PR402_SLA_ESCROW_SETTLE_CRON_DEADLINE_SEC` | 45 | Wall-clock budget per run. Handler stops dispatching new candidates once this elapses; unprocessed candidates are picked up next tick. |
| `PR402_SLA_ESCROW_SETTLE_CRON_COOLDOWN_SEC` | 300 | Minimum seconds between settlement attempts on the same `payment_uid`. Prevents tight retry loops on transient failures. |
| `PR402_SLA_ESCROW_SETTLE_CRON_LOOKBACK_SEC` | 604800 (7 days) | Don't consider `escrow_details` rows older than this. Older payments may have been settled by other actors and pr402 didn't catch the lifecycle. |

All four can be set via Vercel env vars or by inserting rows into the
`parameters` table (params table wins on conflict; env is the
fallback).

## What the SLA-Escrow settlement cron does

The cron leverages sla-escrow program v0.4.0+'s **permissionless
post-outcome settlement**: anyone (including pr402's own fee-payer
keypair, no privileged authority needed) can call `ReleasePayment`
once the oracle approves, or `RefundPayment` once the oracle rejects
or the payment expires without a verdict.

Per cron run:

1. Selects up to `BATCH_LIMIT` funded payments from `escrow_details`
   that haven't been settled per pr402's records and have passed the
   per-row cooldown.
2. Reads each payment's on-chain `Payment` PDA in a single batched
   `getMultipleAccounts` RPC call.
3. For each candidate, applies the v0.4.0 settlement decision matrix:

   | `resolution_state` | `now > expires_at` | `delivery_timestamp` | Action |
   |---|---|---|---|
   | 1 (Approved) | any | any | `ReleasePayment` |
   | 2 (Rejected) | any | any | `RefundPayment` |
   | 0 (Pending) | yes | nonzero | `ReleasePayment` (expired-delivered branch) |
   | 0 (Pending) | yes | 0 | `RefundPayment` (expired-undelivered branch) |
   | 0 (Pending) | no | any | skip (pre-outcome; buyer/seller/admin only) |

4. Builds and submits the appropriate instruction with pr402's keypair
   as fee-payer. Submission is fire-and-forget with preflight; failures
   land back in the candidate set on the next cron tick via the
   cooldown.
5. Records the lifecycle event (`release_payment` / `refund_payment`)
   in `escrow_details` for audit.

The cron NEVER calls `RefundPayment` on the pre-outcome buyer-cooldown
path — that's reserved for buyer / seller / admin per the protocol's
authorization rules. After oracle **rejection**, buyers typically
**self-refund** after `refund_cooldown_seconds` (on-chain config; pr402
does not run a separate refund sweeper). Post-outcome **`RefundPayment`**
(rejection or expired-undelivered) **is** handled by this cron. See
[`oracles/spec/sla-escrow-onchain-abi/v1/NORMATIVE.md`](../../oracles/spec/sla-escrow-onchain-abi/v1/NORMATIVE.md)
§5.4 for the exact program rules and
[`oracles/spec/sla-escrow-protocol/v1/NORMATIVE.md`](../../oracles/spec/sla-escrow-protocol/v1/NORMATIVE.md)
§7 for the actor-level matrix.

## Manual invocation (for testing)

Both crons accept a `POST` companion endpoint that takes optional
overrides via JSON body. Useful for testing or one-shot operator
intervention.

```bash
# SLA-Escrow settlement, dry-run mode (decides per-candidate without submitting any tx):
curl -X POST "https://your-pr402-deployment/api/v1/facilitator/sla-escrow-settle" \
  -H "Authorization: Bearer $CRON_SECRET" \
  -H "Content-Type: application/json" \
  -d '{"limit": 5, "dryRun": true}'

# Real run:
curl "https://your-pr402-deployment/api/v1/facilitator/sla-escrow-settle-cron" \
  -H "Authorization: Bearer $CRON_SECRET"
```

**Manual single payment (no DB row required):** when you have `payment_uid_hex`
but cron skips the row, use the public builder (see
[OPS_RECOVERY_PLAYBOOK.md](./OPS_RECOVERY_PLAYBOOK.md)):

```bash
curl -X POST "https://your-pr402-deployment/api/v1/facilitator/build-sla-escrow-settle-tx" \
  -H "Content-Type: application/json" \
  -d '{"paymentUidHex":"<64-char-lowercase-hex>"}'
```

Response body shape:

```json
{
  "considered": 12,
  "succeeded": 8,
  "skipped": 3,
  "failed": 1,
  "budgetExhaustedRemaining": 0,
  "items": [
    {
      "correlationId": "abc...",
      "paymentUidHex": "0123...",
      "action": "release_payment" | "refund_payment" | "skip_already_settled" | "skip_pre_outcome" | "error",
      "status": "ok" | "skipped" | "dry_run" | "failed",
      "signature": "5ABc..." | null,
      "error": null | "..."
    }
  ]
}
```

## Migration

The SLA-Escrow settlement cron requires a new column on `escrow_details`.
Apply once on existing deployments:

```bash
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/007_escrow_payment_uid.sql
```

Fresh deployments using `init.sql` already include the column; no
separate migration needed.

The migration is forward-compatible: old `escrow_details` rows get
`payment_uid_hex = NULL` and are skipped by the cron's candidate query
(they're presumed to have been funded before the cron existed and may
have been settled by other actors).

## Monitoring

Search Vercel logs for `settlement keeper` (target `server_log`). Each cron tick emits:

| Grep string | Meaning |
|---|---|
| `settlement keeper vault sweep cron started` | Vault sweeper tick began (`task=vault_sweep`) |
| `settlement keeper vault sweep cron finished` | Summary: `scanned`, `attempted`, `succeeded`, `skipped_below_threshold`, `failed` |
| `settlement keeper sla-escrow settle cron started` | Escrow settler tick began (`task=sla_escrow_settle`) |
| `settlement keeper sla-escrow settle cron finished` | Summary: `considered`, `succeeded`, `skipped`, `failed`, `budget_exhausted_remaining` |
| `settlement keeper sla-escrow settle tx submitted` | On-chain `ReleasePayment` / `RefundPayment` landed (includes `signature`) |
| `settlement keeper sla-escrow settle cron deadline reached` | Wall-clock budget hit; remainder deferred to next tick |

Both crons also emit structured logs at `info` level on entry and `info` /
`warn` on per-candidate outcome. Search Vercel logs for:

- `sla-escrow settle cron started` — one log per cron tick with batch
  parameters.
- `sla-escrow settle cron deadline reached` — appears when wall-clock
  budget elapsed before all candidates processed; `remaining` field
  shows how many were deferred to the next tick.
- `apply_escrow_lifecycle_step failed` — non-fatal DB write failure;
  the on-chain tx already landed; next cron will detect from chain
  state and skip.

For sweep-cron, see existing `record_sweep_attempt` / `list_sweep_candidates`
log lines.

If the cron consistently exhausts its deadline, options to investigate
(in order of preference):

1. Increase `PR402_SLA_ESCROW_SETTLE_CRON_BATCH_LIMIT` — more
   candidates per run, same wall-clock.
2. Tighten cron schedule — `*/2 * * * *` instead of `*/5 * * * *`.
3. Increase `PR402_SLA_ESCROW_SETTLE_CRON_DEADLINE_SEC` — but stay well
   below Vercel's 60s function timeout (45 default leaves 15s margin).
