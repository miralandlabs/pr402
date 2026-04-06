
-- Drop all existing tables (clean slate)
DROP TABLE IF EXISTS parameters CASCADE;
DROP TABLE IF EXISTS escrow_lifecycle_events CASCADE;
DROP TABLE IF EXISTS escrow_details CASCADE;
DROP TABLE IF EXISTS payment_attempts CASCADE;
DROP TABLE IF EXISTS resource_providers CASCADE;


-- pr402 facilitator: consolidated PostgreSQL bootstrap (PostgreSQL 14+).
-- Run once: psql "$DATABASE_URL" -f migrations/init.sql
-- Idempotent: CREATE IF NOT EXISTS + parameter seeds use ON CONFLICT DO UPDATE.
--
-- registration_verified_at: set when POST /api/v1/facilitator/onboard succeeds (wallet-signed challenge).
-- GET /api/v1/facilitator/onboard is preview-only and does not write resource_providers.

-- =============================================================================
-- Core: resource providers + payment audit
-- =============================================================================

CREATE TABLE IF NOT EXISTS resource_providers (
    id                  BIGSERIAL PRIMARY KEY,
    wallet_pubkey       TEXT NOT NULL UNIQUE,
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
    completed_at         TIMESTAMPTZ,
    refunded_at          TIMESTAMPTZ,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT escrow_details_one_row_per_payment_attempt UNIQUE (payment_attempt_id)
);

ALTER TABLE escrow_details ENABLE ROW LEVEL SECURITY;

CREATE INDEX IF NOT EXISTS idx_escrow_details_pda ON escrow_details (escrow_pda ASC);
CREATE INDEX IF NOT EXISTS idx_escrow_details_oracle ON escrow_details (oracle_authority ASC);

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
-- =============================================================================

INSERT INTO parameters (param_name, param_value) VALUES
    ('PR402_MAX_DAILY_PROVISION_COUNT', '50'),
    ('PR402_ONBOARD_HMAC_SECRET', 'AgenticEconomics'),
    ('PR402_ONBOARD_CHALLENGE_TTL_SEC', '600'),
    ('PR402_SWEEP_CRON_TOKEN', 'CHANGE_ME_BEFORE_PRODUCTION'),
    ('PR402_SWEEP_CRON_COOLDOWN_SEC', '300'),
    ('PR402_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC', '86400'),
    ('PR402_SWEEP_CRON_BATCH_LIMIT', '50'),
    ('PR402_SWEEP_MIN_SPENDABLE_LAMPORTS', '30000000'),
    (
        'PR402_SWEEP_MIN_SPL_RAW_BY_MINT',
        '{"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v":"3000000","4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU":"3000000","2gNCDGj8Xi9Zs7LNQTPWf4pfZvAM7UHusY4xhKNYg6W6":"3000000"}'
    ),
    ('PR402_SWEEP_MIN_SPL_RAW_DEFAULT', '3000000')
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
