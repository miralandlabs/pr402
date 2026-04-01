-- DEDUPLICATION AND UNIQUE INDEX FOR RESOURCE PROVIDERS
-- 
-- DESCRIPTION:
-- This script ensures that each provider is unique by (wallet_pubkey, settlement_mode, spl_mint).
-- It handles NULLs correctly using the Postgres 15+ NULLS NOT DISTINCT feature.

BEGIN;

-- 1. Deduplicate by keeping only the most recent entry for each unique provider rail
DELETE FROM resource_providers 
WHERE id NOT IN (
    SELECT MAX(id) 
    FROM resource_providers 
    GROUP BY wallet_pubkey, settlement_mode, spl_mint
);

-- 2. Drop any conflicting legacy generic indexes
DROP INDEX IF EXISTS idx_resource_providers_wallet;

-- 3. Create THE HIGH-FIDELITY NULL-SAFE UNIQUE INDEX
-- This ensures that (Alice, 'spl', NULL) == (Alice, 'spl', NULL)
CREATE UNIQUE INDEX IF NOT EXISTS idx_resource_providers_dedup_trip 
ON resource_providers (wallet_pubkey, settlement_mode, spl_mint) 
NULLS NOT DISTINCT;

COMMIT;

-- 4. Verification
SELECT id, wallet_pubkey, settlement_mode, spl_mint, updated_at 
FROM resource_providers 
ORDER BY updated_at DESC;
