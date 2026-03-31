-- pr402: append-only SLA escrow lifecycle audit (Phase 3)
--
-- One row per on-chain step after FundPayment (submit-delivery, confirm-oracle, …).
-- Apply: psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/003_escrow_lifecycle_events.sql

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
