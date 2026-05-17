---
name: Register an oracle on /capabilities
about: Request that pr402 advertise your running oracle under slaEscrowOracleProfiles[]
title: "[oracle-register] <profile> — <operator-pubkey-prefix>"
labels: ["oracle-registration", "needs-review"]
assignees: []
---

<!--
  Use this template after you have:
  1. Deployed an oracle (any of the three reference families, or your own
     custom evaluator) and confirmed it is healthy.
  2. Run `bash oracles/scripts/announce-to-pr402.sh https://your-oracle...`
     to generate the SQL block this template asks for.

  The pr402 operator reviews each request before running the SQL against
  pr402's `parameters` table. Once accepted, your oracle appears on
  GET /api/v1/facilitator/capabilities → slaEscrowOracleProfiles[]
  within ~60 seconds (parameters cache TTL).
-->

## 1. Oracle endpoint

**Base URL:** `https://...`

Anyone reviewing this issue should be able to fetch the following with no
auth:

- [ ] `curl -fsS <base>/health` returns `200` with `chain_connected=true`
      and `websocket_connected=true`.
- [ ] `curl -fsS <base>/v1/registry/info` returns the registered profile.
- [ ] `curl -fsS <base>/v1/registry/info | jq -r .oraclePubkey` matches
      the operator pubkey below.

## 2. Registration data

Output of `announce-to-pr402.sh` against the endpoint above. **Paste
verbatim** — the operator runs this against pr402's Postgres on accept:

```sql
-- paste here
```

## 3. Operator attestations

- **Operator (organisation / team):**
- **Primary contact for incidents (email or matrix):**
- **Keypair custody (hot / warm + cold backups, single signer or
  multi-sig, who has access):**
- **Uptime SLO you commit to:** e.g. 99.5% monthly
- **Funding plan for SOL settlement fees:** e.g. "Hot wallet auto-tops at
  0.5 SOL floor, alerts at 0.2 SOL"
- **Devnet evidence link** (transcripts / journalctl / oracle_jobs SELECT
  output from a successful end-to-end settlement) — required for
  Mainnet listing, optional for Devnet listing:

## 4. Profile-specific attestations

### If `x402/oracles/api-quality/v1`:

- **Strict-profile mode enabled** (`ORACLE_STRICT_PROFILE=true`)?
  - [ ] Yes
  - [ ] No (please justify)

### If `x402/oracles/onchain-transfer/v1`:

- **Pinned cluster** (`TRANSFER_CLUSTER`):
  - [ ] mainnet-beta
  - [ ] devnet
  - [ ] testnet
- **RPC provider for `getTransaction(jsonParsed)`:** (provider name; this
  helps the operator predict rate-limit incidents)

### If `x402/oracles/file-delivery/attestation/v1`:

- **Storage backend:**
  - [ ] MinIO (self-hosted)
  - [ ] AWS S3
  - [ ] Cloudflare R2
  - [ ] Backblaze B2 / Wasabi / other (specify)
- **Size cap configured** (`ORACLE_REGISTRY_MAX_BLOB_BYTES`):

### If a custom profile:

- **Profile id you'd like to register:** `x402/oracles/<your-domain>/v1`
- **Normative spec URL** (NORMATIVE.md or equivalent):
- **Why this domain isn't covered by an existing profile:** (one
  paragraph)
- **Are you open to publishing the evaluator source?** Trust ↑ when yes.

## 5. Acknowledgements

- [ ] I understand listing is editorial, not self-service. The pr402
      operator may reject or delay this request without compensation.
- [ ] I understand listing can be revoked if my oracle becomes unhealthy
      or the operator behaviour changes.
- [ ] I understand `defaultOperatorPubkey` is a **hint** to buyers — it
      does not bind any particular `oracle_authority` on-chain.
- [ ] I have read [`oracle-common/docs/PR402_CONTRACT.md`](https://github.com/miraland-labs/oracles/blob/main/oracle-common/docs/PR402_CONTRACT.md)
      and the
      [Oracle Developer Guide](https://github.com/miraland-labs/oracles/blob/main/docs/marketing/oracle-intro-article.md).

## 6. Anything else?

(Optional — known issues, planned upgrades, requests for the operator,
links to past audits, etc.)
