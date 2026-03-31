-- pr402: one escrow_details row per payment_attempt (Phase 0 full-cycle prep)
--
-- Previously escrow_pda was UNIQUE, so repeat payments on the same mint/escrow overwrote
-- a single row. Uniqueness moves to payment_attempt_id so each fund has its own audit row.
--
-- Apply: psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/002_escrow_details_one_per_payment.sql

-- Drop uniqueness on escrow_pda (keep non-unique index from idx_escrow_details_pda if present).
ALTER TABLE escrow_details DROP CONSTRAINT IF EXISTS escrow_details_escrow_pda_key;

-- One detail row per payment attempt (verify + settle upserts merge on this key).
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'escrow_details_one_row_per_payment_attempt'
    ) THEN
        ALTER TABLE escrow_details
            ADD CONSTRAINT escrow_details_one_row_per_payment_attempt
            UNIQUE (payment_attempt_id);
    END IF;
END $$;
