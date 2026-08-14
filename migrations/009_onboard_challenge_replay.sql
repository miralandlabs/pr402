-- One-time use for wallet-signed seller/resource administration challenges.
CREATE TABLE IF NOT EXISTS onboard_challenge_uses (
    challenge_sha256    TEXT PRIMARY KEY,
    wallet_pubkey       TEXT NOT NULL,
    used_at             TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE onboard_challenge_uses ENABLE ROW LEVEL SECURITY;

CREATE INDEX IF NOT EXISTS idx_onboard_challenge_uses_used_at
    ON onboard_challenge_uses (used_at);
