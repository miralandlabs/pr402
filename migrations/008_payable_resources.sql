-- Layer 3: one row per distinct paid HTTP resource (PaymentRequired.resource.url).
-- Additive only; does not alter resource_providers columns.
--
-- Fresh installs: included in migrations/init.sql (run init.sql once).
-- Existing deployments: apply this file incrementally after 007_escrow_payment_uid.sql.

CREATE TABLE IF NOT EXISTS payable_resources (
    id                          BIGSERIAL PRIMARY KEY,

    wallet_pubkey               TEXT NOT NULL,
    resource_provider_id        BIGINT REFERENCES resource_providers(id) ON DELETE SET NULL,

    resource_url                TEXT NOT NULL,
    http_method                 TEXT NOT NULL DEFAULT 'GET',
    seller_resource_id          TEXT,

    title                       TEXT NOT NULL,
    description                 TEXT,
    use_case                    TEXT,
    category                    TEXT,
    tags                        TEXT[],

    scheme                      TEXT NOT NULL,
    network                     TEXT,
    intent_contract_url         TEXT,
    facilitator_hint            TEXT,

    listing_opt_in              BOOLEAN NOT NULL DEFAULT FALSE,
    registration_verified_at    TIMESTAMPTZ,
    verified_schema_version     INTEGER NOT NULL DEFAULT 1,
    inactive                    BOOLEAN NOT NULL DEFAULT FALSE,
    retired_at                  TIMESTAMPTZ,

    source                      TEXT NOT NULL DEFAULT 'register_ui',

    manifest_origin             TEXT,
    manifest_sha256             TEXT,

    last_probe_at               TIMESTAMPTZ,
    last_probe_ok               BOOLEAN,
    last_probe_error            TEXT,
    last_probe_http_status      INTEGER,
    last_probe_scheme           TEXT,

    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT payable_resources_scheme_check
        CHECK (scheme IN ('exact', 'sla-escrow'))
);

ALTER TABLE payable_resources ENABLE ROW LEVEL SECURITY;

CREATE UNIQUE INDEX IF NOT EXISTS idx_payable_resources_resource_url
    ON payable_resources (resource_url);

CREATE UNIQUE INDEX IF NOT EXISTS idx_payable_resources_wallet_slug
    ON payable_resources (wallet_pubkey, seller_resource_id)
    WHERE seller_resource_id IS NOT NULL AND retired_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_payable_resources_public_listing
    ON payable_resources (updated_at DESC)
    WHERE listing_opt_in = TRUE
      AND registration_verified_at IS NOT NULL
      AND inactive = FALSE
      AND retired_at IS NULL
      AND last_probe_ok = TRUE;

CREATE INDEX IF NOT EXISTS idx_payable_resources_wallet_active
    ON payable_resources (wallet_pubkey, updated_at DESC)
    WHERE retired_at IS NULL;
