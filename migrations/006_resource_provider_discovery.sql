-- pr402 migration 006 — seller discovery surface + retirement flag + schema version.
--
-- Idempotent: safe to run on deployed databases that already have the base `init.sql` schema.
-- New columns are all nullable or have sensible defaults, so existing rows remain valid.
--
-- Discovery columns added to `resource_providers`:
--   service_url            — seller's 402-gated endpoint. Required for public listing.
--   display_name           — human-friendly name, ≤ 64 chars (enforced in application layer).
--   description            — tweet-sized blurb, ≤ 280 chars (enforced in application layer).
--   tags                   — seller-declared tags (up to 5, lowercase/[a-z0-9-]).
--   service_metadata       — opaque JSONB (≤ 4 KB, enforced in application layer).
--   listing_opt_in         — must be TRUE to appear on public GET /providers.
--   retired_at             — when set, the row is retired; registry reads skip it.
--   verified_schema_version — which version of the registration contract produced this row
--                             (so we can detect stale rows if the signed payload shape evolves).

ALTER TABLE resource_providers
    ADD COLUMN IF NOT EXISTS service_url             TEXT,
    ADD COLUMN IF NOT EXISTS display_name            TEXT,
    ADD COLUMN IF NOT EXISTS description             TEXT,
    ADD COLUMN IF NOT EXISTS tags                    TEXT[],
    ADD COLUMN IF NOT EXISTS service_metadata        JSONB,
    ADD COLUMN IF NOT EXISTS listing_opt_in          BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS retired_at              TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS verified_schema_version INTEGER NOT NULL DEFAULT 1;

-- Public discovery lookups filter on these three predicates together; a partial index
-- keeps scans cheap as the registry grows.
CREATE INDEX IF NOT EXISTS idx_resource_providers_public_listing
    ON resource_providers (updated_at DESC)
    WHERE listing_opt_in = TRUE
      AND registration_verified_at IS NOT NULL
      AND inactive = FALSE
      AND retired_at IS NULL;

-- Backward-compatible view of "is this row public-visible" for ad-hoc ops queries.
COMMENT ON COLUMN resource_providers.service_url IS
    'Seller-declared 402-gated endpoint URL. Required for the row to appear in the public /providers listing.';
COMMENT ON COLUMN resource_providers.listing_opt_in IS
    'Seller must explicitly opt in to appear in the public /providers listing. Default FALSE.';
COMMENT ON COLUMN resource_providers.retired_at IS
    'When non-NULL, the row is retired: excluded from discovery, and POST /onboard refuses to reuse it.';
