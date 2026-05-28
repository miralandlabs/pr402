
-- Drop all existing tables (clean slate)
DROP TABLE IF EXISTS parameters CASCADE;
DROP TABLE IF EXISTS escrow_lifecycle_events CASCADE;
DROP TABLE IF EXISTS escrow_details CASCADE;
DROP TABLE IF EXISTS payment_attempts CASCADE;
DROP TABLE IF EXISTS resource_providers CASCADE;


-- pr402 facilitator: consolidated PostgreSQL bootstrap (PostgreSQL 15+; NULLS NOT DISTINCT).
-- Run once: psql "$DATABASE_URL" -f migrations/init.sql
-- Idempotent: CREATE IF NOT EXISTS + parameter seeds use ON CONFLICT DO UPDATE.
--
-- registration_verified_at: set when POST /api/v1/facilitator/sellers/{wallet}/register succeeds (wallet-signed challenge).
-- GET /api/v1/facilitator/sellers/{wallet}/preview is preview-only and does not write resource_providers.

-- =============================================================================
-- Core: resource providers + payment audit
-- =============================================================================

CREATE TABLE IF NOT EXISTS resource_providers (
    id                  BIGSERIAL PRIMARY KEY,
    wallet_pubkey       TEXT NOT NULL,
    -- native_sol | spl (one settlement rail per row; spl_mint set when spl)
    settlement_mode     TEXT NOT NULL DEFAULT 'native_sol',
    spl_mint            TEXT,
    split_vault_pda     TEXT,
    vault_sol_storage_pda TEXT,
    sweep_threshold     BIGINT,
    registration_verified_at TIMESTAMPTZ,
    last_sweep_attempt_at TIMESTAMPTZ,
    last_sweep_signature TEXT,
    inactive            BOOLEAN NOT NULL DEFAULT FALSE,
    -- Retirement (opt-out): set by POST /onboard/retire so the row is excluded from
    -- public discovery and future signed submits refuse to reuse it.
    retired_at          TIMESTAMPTZ,
    -- Discovery surface (populated via optional `discovery` sub-object on POST /onboard;
    -- application layer enforces length / pattern limits).
    service_url         TEXT,
    display_name        TEXT,
    description         TEXT,
    tags                TEXT[],
    service_metadata    JSONB,
    listing_opt_in      BOOLEAN NOT NULL DEFAULT FALSE,
    -- Versioning for the signed-onboard payload contract (e.g. bumped when the required
    -- discovery fields grow). Lets ops queries spot stale verified rows.
    verified_schema_version INTEGER NOT NULL DEFAULT 1,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE resource_providers ENABLE ROW LEVEL SECURITY;

-- Create THE HIGH-FIDELITY NULL-SAFE UNIQUE INDEX
-- This ensures that (Alice, 'spl', NULL) == (Alice, 'spl', NULL)
CREATE UNIQUE INDEX IF NOT EXISTS idx_resource_providers_dedup_trip 
ON resource_providers (wallet_pubkey, settlement_mode, spl_mint) 
NULLS NOT DISTINCT;

CREATE INDEX IF NOT EXISTS idx_resource_providers_created_at
    ON resource_providers (created_at ASC);

CREATE INDEX IF NOT EXISTS idx_resource_providers_updated_at
    ON resource_providers (updated_at ASC);

-- Public discovery lookups filter on four predicates together; a partial index keeps
-- scans cheap as the registry grows.
CREATE INDEX IF NOT EXISTS idx_resource_providers_public_listing
    ON resource_providers (updated_at DESC)
    WHERE listing_opt_in = TRUE
      AND registration_verified_at IS NOT NULL
      AND inactive = FALSE
      AND retired_at IS NULL;

CREATE TABLE IF NOT EXISTS payment_attempts (
    id                   BIGSERIAL PRIMARY KEY,
    correlation_id       TEXT NOT NULL UNIQUE,
    resource_provider_id BIGINT REFERENCES resource_providers (id) ON DELETE SET NULL,
    verify_at            TIMESTAMPTZ,
    verify_ok            BOOLEAN,
    verify_error         TEXT,
    settle_at            TIMESTAMPTZ,
    settle_ok            BOOLEAN,
    settle_error         TEXT,
    settlement_signature TEXT,
    payer_wallet         TEXT,
    scheme               TEXT,
    amount               TEXT,
    asset                TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE payment_attempts ENABLE ROW LEVEL SECURITY;

CREATE INDEX IF NOT EXISTS idx_payment_attempts_provider
    ON payment_attempts (resource_provider_id ASC);

CREATE INDEX IF NOT EXISTS idx_payment_attempts_scheme
    ON payment_attempts (scheme ASC);

-- =============================================================================
-- SLAEscrow: multi-step institutional audit
-- =============================================================================
-- escrow_details: one row per payment_attempt (UNIQUE on payment_attempt_id).
-- escrow_pda is NOT unique — many funded payments share the same escrow PDA (mint rail).
-- Application upsert: ON CONFLICT (payment_attempt_id) (see Pr402Db::upsert_escrow_detail).

CREATE TABLE IF NOT EXISTS escrow_details (
    id                   BIGSERIAL PRIMARY KEY,
    payment_attempt_id   BIGINT NOT NULL REFERENCES payment_attempts (id) ON DELETE CASCADE,
    escrow_pda           TEXT NOT NULL,
    bank_pda             TEXT NOT NULL,
    oracle_authority     TEXT NOT NULL,
    fund_signature       TEXT,
    delivery_signature   TEXT,
    resolution_signature TEXT,
    resolution_state     SMALLINT DEFAULT 0, -- 0: Pending, 1: Approved, 2: Denied
    sla_hash             TEXT,
    delivery_hash        TEXT,
    payment_uid_hex      TEXT, -- 64 lowercase hex (32-byte on-chain Payment.payment_uid); NULL for legacy rows
    completed_at         TIMESTAMPTZ,
    refunded_at          TIMESTAMPTZ,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT escrow_details_one_row_per_payment_attempt UNIQUE (payment_attempt_id)
);

ALTER TABLE escrow_details ENABLE ROW LEVEL SECURITY;

CREATE INDEX IF NOT EXISTS idx_escrow_details_pda ON escrow_details (escrow_pda ASC);
CREATE INDEX IF NOT EXISTS idx_escrow_details_oracle ON escrow_details (oracle_authority ASC);
CREATE INDEX IF NOT EXISTS idx_escrow_details_payment_uid
    ON escrow_details (payment_uid_hex ASC)
    WHERE payment_uid_hex IS NOT NULL;

-- Append-only lifecycle steps after FundPayment (see Pr402Db::apply_escrow_lifecycle_step).

CREATE TABLE IF NOT EXISTS escrow_lifecycle_events (
    id                   BIGSERIAL PRIMARY KEY,
    payment_attempt_id   BIGINT NOT NULL REFERENCES payment_attempts (id) ON DELETE CASCADE,
    step                 TEXT NOT NULL,
    tx_signature         TEXT,
    payload              JSONB,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE escrow_lifecycle_events ENABLE ROW LEVEL SECURITY;

CREATE INDEX IF NOT EXISTS idx_escrow_lifecycle_events_attempt
    ON escrow_lifecycle_events (payment_attempt_id ASC, created_at ASC);

CREATE INDEX IF NOT EXISTS idx_escrow_lifecycle_events_step
    ON escrow_lifecycle_events (step ASC);

-- Application sets payment_attempts.updated_at on UPDATE (avoids PG trigger dialect drift).

-- =============================================================================
-- parameters (key/value; DB overrides env for long or frequently rotated values)
--
-- Do NOT store PR402_PARAMETERS_CACHE_TTL_SEC here: pr402 reads that only from the
-- process environment (e.g. Vercel env / .env). It is not read from this table.
-- =============================================================================

CREATE TABLE IF NOT EXISTS parameters (
    id             BIGSERIAL PRIMARY KEY,
    param_name     TEXT NOT NULL,
    param_value    TEXT NOT NULL,
    inactive       BOOLEAN NOT NULL DEFAULT FALSE,
    effective_from TIMESTAMPTZ,
    expires_at     TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE parameters ENABLE ROW LEVEL SECURITY;

CREATE UNIQUE INDEX IF NOT EXISTS uniq_parameters_param_name ON parameters (param_name ASC);

CREATE INDEX IF NOT EXISTS idx_parameters_inactive ON parameters (inactive ASC);

-- =============================================================================
-- Default parameter seeds (safe to re-run).
--
-- Notes (DB parameter precedence > env in this project):
-- - PR402_SWEEP_CRON_TOKEN:
--     Bearer token required by POST /api/v1/facilitator/sweep.
--     Default below is a bootstrap placeholder; rotate immediately in production.
-- - PR402_SWEEP_CRON_COOLDOWN_SEC (default 300):
--     Min seconds between sweep attempts per provider rail.
-- - PR402_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC (default 86400):
--     Candidate must have a successful settle within this window.
-- - PR402_SWEEP_CRON_BATCH_LIMIT (default 50):
--     Max provider rails processed per cron run.
-- - PR402_SWEEP_MIN_SPENDABLE_LAMPORTS (default 30000000):
--     SOL floor (0.03 SOL) before sweep.
-- - PR402_SWEEP_MIN_SPL_RAW_DEFAULT (default 3000000):
--     SPL raw floor fallback (e.g. 3.0 @ 6 decimals).
-- - PR402_SWEEP_MIN_SPL_RAW_BY_MINT:
--     Optional per-mint SPL raw floor map.
-- - PR402_ALLOWED_PAYMENT_MINTS:
--     Comma- or whitespace-separated base58 mint pubkeys. Parsed in Rust by splitting on commas
--     and ASCII whitespace (see `parameters::resolve_allowed_payment_mints`).
--     Include native SOL explicitly as 11111111111111111111111111111111 (matches x402/Solana
--     convention for `asset` on native SOL lines).
--     Use exactly one USDC mint for your cluster (mainnet EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v;
--     devnet 4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU). Do not mix cluster mints on one deploy.
--     Empty / omitted = allowlist disabled (permissive; not for production).
-- =============================================================================

INSERT INTO parameters (param_name, param_value) VALUES
    ('PR402_MAX_DAILY_PROVISION_COUNT', '50'),
    ('PR402_ONBOARD_HMAC_SECRET', 'AgenticEconomics'),
    ('PR402_ONBOARD_CHALLENGE_TTL_SEC', '600'),
    (
        'PR402_ALLOWED_PAYMENT_MINTS',
        '11111111111111111111111111111111,EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'
    ),
    -- Hard-gate: refuse to start when PR402_ALLOWED_PAYMENT_MINTS is empty. Seeds `false`
    -- for backward compatibility with existing devnet deployments; flip to `true` in
    -- production to prevent the silent "any mint is accepted" trap on misconfiguration.
    ('PR402_REQUIRE_MINT_ALLOWLIST', 'false'),
    ('PR402_SWEEP_CRON_TOKEN', 'SHARE_SAME_VALUE_BTW_CRON_SECRET_AND_CRON_TOKEN'),
    ('PR402_SWEEP_CRON_COOLDOWN_SEC', '300'),
    ('PR402_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC', '86400'),
    ('PR402_SWEEP_CRON_BATCH_LIMIT', '50'),
    ('PR402_SWEEP_MIN_SPENDABLE_LAMPORTS', '30000000'),
    (
        'PR402_SWEEP_MIN_SPL_RAW_BY_MINT',
        '{"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v":"3000000"}'
    ),
    ('PR402_SWEEP_MIN_SPL_RAW_DEFAULT', '3000000'),
    -- SLA-Escrow strict cross-check at build time. When 'true', /build-sla-escrow-payment-tx
    -- rejects requests whose chosen oracleAuthority resolves to a profileId not advertised
    -- on /capabilities. Seeds 'false' so existing sellers that haven't migrated to the
    -- richer oracleProfiles[] shape keep working unchanged. See pr402/.env.example.
    ('PR402_SLA_ESCROW_REQUIRE_PROFILE_MATCH', 'false'),
    -- Wave A §3.2 oracle health gate. When 'true', pr402 probes each advertised oracle's
    -- /health endpoint (derived from its registry_url, 30s cache, 2s timeout) before
    -- (a) advertising the profile on /capabilities (unhealthy → annotated, not removed)
    -- and (b) building an SLA-Escrow FundPayment for that profile (unhealthy → 503).
    -- Seeds 'false' so the gate is dormant until all advertised oracles publish a
    -- registry_url and reliably respond 200 on /health. Flip to 'true' to activate.
    ('PR402_SLA_ESCROW_REQUIRE_ORACLE_HEALTHY', 'false'),
    -- FundPayment TTL floor (x402/sla-escrow-fund-payment-ttl/v1): min maxTimeoutSeconds =
    -- delivery_cutoff + delivery_budget. Tune here to avoid Vercel env size limits.
    ('PR402_SLA_ESCROW_DELIVERY_CUTOFF_SECONDS', '300'),
    ('PR402_SLA_ESCROW_DELIVERY_BUDGET_SECONDS', '300'),
    ('PR402_SLA_ESCROW_SETTLE_CRON_TOKEN', 'SHARE_SAME_VALUE_BTW_CRON_SECRET_AND_CRON_TOKEN'),
    ('PR402_SLA_ESCROW_SETTLE_CRON_COOLDOWN_SEC', '300'),
    ('PR402_SLA_ESCROW_SETTLE_CRON_BATCH_LIMIT', '15'),
    ('PR402_SLA_ESCROW_SETTLE_CRON_DEADLINE_SEC', '45'),
    ('PR402_SLA_ESCROW_SETTLE_CRON_LOOKBACK_SEC', '604800')
ON CONFLICT (param_name) DO UPDATE SET
    param_value = EXCLUDED.param_value,
    updated_at = NOW();

-- =============================================================================
-- Examples (uncomment / adjust after deploy)
-- =============================================================================
-- INSERT INTO parameters (param_name, param_value) VALUES
--   ('PR402_ONBOARD_HMAC_SECRET', 'generate-a-long-random-secret'),
--   ('PR402_ONBOARD_CHALLENGE_TTL_SEC', '600'),
--   ('PR402_QUOTE_SPL_MINTS', 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v,Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB')
-- ON CONFLICT (param_name) DO UPDATE SET
--   param_value = EXCLUDED.param_value,
--   updated_at = NOW();

-- =============================================================================
-- SLA-Escrow oracle profiles (advertised on GET /capabilities as `slaEscrowOracleProfiles[]`)
--
-- Two configuration modes (mutually exclusive — JSON wins when set):
--
-- 1. Full JSON override:
-- INSERT INTO parameters (param_name, param_value) VALUES
--   (
--     'PR402_SLA_ESCROW_ORACLE_PROFILES_JSON',
--     '[{"profileId":"x402/oracles/api-quality/v1","defaultOperatorPubkey":"OracleAuthorityPubkey..."},
--       {"profileId":"x402/oracles/onchain-transfer/v1","defaultOperatorPubkey":"OracleAuthorityPubkey..."},
--       {"profileId":"x402/oracles/file-delivery/attestation/v1","defaultOperatorPubkey":"OracleAuthorityPubkey..."}]'
--   )
-- ON CONFLICT (param_name) DO UPDATE SET
--   param_value = EXCLUDED.param_value,
--   updated_at = NOW();
--
-- 2. Ergonomic per-profile keys (each profile only emitted when its DEFAULT_PUBKEY is set):
-- INSERT INTO parameters (param_name, param_value) VALUES
--   ('PR402_SLA_ESCROW_API_QUALITY_DEFAULT_PUBKEY',          '<api-quality oracle pubkey>'),
--   ('PR402_SLA_ESCROW_API_QUALITY_NORMATIVE_SPEC_URL',      'https://github.com/miraland-labs/oracles/blob/main/oracle-api-quality/spec/api-quality-v1/NORMATIVE.md'),
--   ('PR402_SLA_ESCROW_API_QUALITY_REGISTRY_URL',            'https://oracle-api.example.com/v1/registry'),
--   ('PR402_SLA_ESCROW_API_QUALITY_EVIDENCE_REGISTRY_NOTE',  'Sellers POST hash-bound SLA + delivery JSON'),
--   ('PR402_SLA_ESCROW_ONCHAIN_TRANSFER_DEFAULT_PUBKEY',     '<onchain-transfer oracle pubkey>'),
--   ('PR402_SLA_ESCROW_FILE_DELIVERY_DEFAULT_PUBKEY',        '<file-delivery oracle pubkey>')
-- ON CONFLICT (param_name) DO UPDATE SET
--   param_value = EXCLUDED.param_value,
--   updated_at = NOW();
--
-- 3. Activate the security gates once your oracles are live and reliable
--    (each defaults to 'false' in the active seed above; uncomment to flip ON):
-- INSERT INTO parameters (param_name, param_value) VALUES
--   ('PR402_SLA_ESCROW_REQUIRE_PROFILE_MATCH',  'true'),
--   ('PR402_SLA_ESCROW_REQUIRE_ORACLE_HEALTHY', 'true')
-- ON CONFLICT (param_name) DO UPDATE SET
--   param_value = EXCLUDED.param_value,
--   updated_at = NOW();

-- 4. Oracle authorities (comma-separated list; no whitespace; order not important):
-- INSERT INTO parameters (param_name, param_value)
-- VALUES ('PR402_ORACLE_AUTHORITIES',
--         'FaciLFwHjbW9V1PtF3vAweL1K1hgin9mvXNXatEQKJdu,oraG62Mr5hDYeSbAtKMpEYFw22SLpZdebXvDe2Qr7xV')
-- ON CONFLICT (param_name) DO UPDATE
--   SET param_value = EXCLUDED.param_value, updated_at = NOW();

-- =============================================================================

