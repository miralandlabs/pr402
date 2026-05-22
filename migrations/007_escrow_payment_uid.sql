-- pr402: add `payment_uid` to `escrow_details` so the SLA-Escrow settlement
-- cron can enumerate Payment PDAs without re-decoding the FundPayment
-- instruction. Backwards-compatible (NULLABLE column).
--
-- Apply: psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/007_escrow_payment_uid.sql

ALTER TABLE escrow_details
    ADD COLUMN IF NOT EXISTS payment_uid_hex TEXT;

-- Index speeds up the cron candidate query, which filters by lifecycle state
-- and orders by `updated_at`. `payment_uid_hex` is also useful for ad-hoc
-- operator lookups by uid.
CREATE INDEX IF NOT EXISTS idx_escrow_details_payment_uid
    ON escrow_details (payment_uid_hex ASC)
    WHERE payment_uid_hex IS NOT NULL;
