-- pr402 facilitator: migration 001 - Enriched Payment & SLAEscrow Support
--
-- Goals:
-- 1. Extend `payment_attempts` with common metadata (scheme, amount, asset, payer).
-- 2. Add `escrow_details` for multi-step SLAEscrow lifecycle tracking.
--

-- 1. Enhance payment_attempts (The Universal Layer)
ALTER TABLE payment_attempts 
    ADD COLUMN IF NOT EXISTS payer_wallet  TEXT,
    ADD COLUMN IF NOT EXISTS scheme        TEXT,
    ADD COLUMN IF NOT EXISTS amount        TEXT,
    ADD COLUMN IF NOT EXISTS asset         TEXT;

CREATE INDEX IF NOT EXISTS idx_payment_attempts_scheme ON payment_attempts (scheme ASC);

-- 2. Create escrow_details (The Specialized Layer)
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
