-- 005: Per-merchant sweep threshold (raw lamports / SPL raw units).
-- NULL = use global PR402_SWEEP_MIN_* parameters (backward compatible).
ALTER TABLE resource_providers ADD COLUMN IF NOT EXISTS sweep_threshold BIGINT;
