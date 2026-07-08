//! Simplified configuration for Vercel Serverless environment.
//!
//! Uses environment variables only - no JSON config files needed.

use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use url::Url;

use crate::chain::ChainId;
use crate::parameters;
use sla_escrow_api::state::Bank as EscrowBank;
use solana_pubkey::Pubkey;
use universalsettle_api::state::Config as USConfig;

/// Facilitator configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Solana RPC endpoint URL
    pub solana_rpc_url: Url,
    /// Solana WebSocket pubsub URL (optional)
    pub solana_pubsub_url: Option<Url>,
    /// Solana chain ID (CAIP-2 format, e.g., "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp")
    pub chain_id: ChainId,
    /// Fee payer private key (base58 encoded, 64 bytes)
    pub fee_payer_private_key: String,
    /// UniversalSettle configuration (optional)
    pub universalsettle: Option<UniversalSettleConfig>,
    /// SLAEscrow configuration (optional)
    pub escrow: Option<SLAEscrowConfig>,
    /// When true, HTTP `POST .../build-sla-escrow-payment-tx` accepts `facilitatorPaysTransactionFees: true`.
    /// Default false. Env: `PR402_SLA_ESCROW_ALLOW_FACILITATOR_FEE_SPONSORSHIP` (`1` / `true` / `yes`).
    pub sla_escrow_allow_facilitator_fee_sponsorship: bool,
    /// Challenge validity window (seconds). Default 600; max 3600 enforced in handlers. DB override: `parameters` / `PR402_ONBOARD_CHALLENGE_TTL_SEC`.
    pub onboard_challenge_ttl_sec: u64,
}

/// On-chain UniversalSettle `Config` params, read from the Config PDA.
#[derive(Debug, Clone, Copy)]
pub struct UsOnchainParams {
    pub fee_destination: Pubkey,
    pub fee_bps: u16,
    pub min_fee_amount: u64,
    pub min_fee_amount_sol: u64,
}

/// UniversalSettle configuration for fee-charging facilitator.
///
/// `program_id` is known from the environment; the fee destination / bps / floors are read
/// from the on-chain Config PDA and cached **lazily** in `onchain`. A failed load is never
/// cached, so the next hot-path call retries — a cold-start RPC blip cannot degrade the
/// whole (serverless) instance for its lifetime.
#[derive(Debug, Clone)]
pub struct UniversalSettleConfig {
    /// UniversalSettle program ID
    pub program_id: Pubkey,
    /// On-chain Config params, populated on first successful load. `Arc<OnceLock<_>>` only so the
    /// struct can keep `#[derive(Clone)]` (required because `Config` derives `Clone` and is cloned
    /// once at startup). The cache is meaningful per **warm process**: the provider is a
    /// process-global static reused across Vercel invocations, so the load runs once per cold start
    /// (or lazily once if boot's warm-up failed) and every later request on that instance reuses it.
    onchain: Arc<OnceLock<UsOnchainParams>>,
}

/// SLAEscrow configuration for escrow-based settlements.
#[derive(Debug, Clone)]
pub struct SLAEscrowConfig {
    /// SLAEscrow program ID
    pub program_id: Pubkey,
    /// Bank PDA (read from chain or derived)
    pub bank_address: Option<Pubkey>,
    pub fee_bps: Option<u16>,
    /// Oracle fee basis points (read from on-chain Escrow account, default 0)
    pub oracle_fee_bps: Option<u16>,
    /// List of trusted oracle authorities as candidates for user selection
    pub oracle_authorities: Vec<Pubkey>,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Required environment variables:
    /// - `SOLANA_RPC_URL`: Solana RPC endpoint (e.g., "https://api.mainnet-beta.solana.com")
    /// - `SOLANA_CHAIN_ID`: Chain ID in CAIP-2 format (e.g., "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp")
    /// - `FEE_PAYER_PRIVATE_KEY`: Base58-encoded private key for fee payer
    ///
    /// Optional environment variables:
    /// - `DATABASE_URL`: PostgreSQL connection string; enables persistence (`migrations/init.sql`).
    /// - `PR402_ONBOARD_HMAC_SECRET`: Signed onboard HMAC key; may use `parameters` table instead (see `migrations/init.sql`, [`crate::parameters`]).
    /// - `PR402_ONBOARD_CHALLENGE_TTL_SEC`: Challenge lifetime (default 600).
    /// - `PR402_PARAMETERS_CACHE_TTL_SEC`: Per-process cache TTL for Postgres `parameters` reads (default 60); **env only**, not read from the `parameters` row.
    /// - `SOLANA_PUBSUB_URL`: WebSocket URL for pubsub (default: None)
    /// - `UNIVERSALSETTLE_PROGRAM_ID`: Enables `v2:solana:exact` with UniversalSettle (vault, fees, sweep). Omit only if you do not serve that scheme.
    /// - `ESCROW_PROGRAM_ID`: Registers `v2:solana:sla-escrow`. Omit if you do not serve escrow. At least one settlement program should match what RPs advertise.
    /// - `PR402_ORACLE_AUTHORITIES` (DB row preferred — see [`crate::parameters::PR402_ORACLE_AUTHORITIES`]) **or** legacy `ORACLE_AUTHORITIES` env: comma-separated allow-list of trusted oracle authority pubkeys for the SLA-Escrow scheme. The DB row beats the env var so operators can grow the list past Vercel's env-size limit; both fall back to an empty list.
    /// - `PR402_SLA_ESCROW_ALLOW_FACILITATOR_FEE_SPONSORSHIP`: If `1`/`true`/`yes`, clients may request facilitator-paid Solana fees on SLA-Escrow build (`facilitatorPaysTransactionFees: true`). Default: disabled (such requests return 400).
    pub fn from_env() -> Result<Self, ConfigError> {
        let solana_rpc_url = std::env::var("SOLANA_RPC_URL")
            .map_err(|_| ConfigError::MissingEnvVar("SOLANA_RPC_URL"))?
            .parse::<Url>()
            .map_err(|e: url::ParseError| {
                ConfigError::InvalidUrl("SOLANA_RPC_URL", e.to_string())
            })?;

        let solana_pubsub_url = std::env::var("SOLANA_PUBSUB_URL")
            .ok()
            .and_then(|s| s.parse().ok());

        let chain_id_str = std::env::var("SOLANA_CHAIN_ID")
            .map_err(|_| ConfigError::MissingEnvVar("SOLANA_CHAIN_ID"))?;
        let chain_id = ChainId::from_str(&chain_id_str)
            .map_err(|e| ConfigError::InvalidChainId(chain_id_str, e.to_string()))?;

        let fee_payer_private_key = std::env::var("FEE_PAYER_PRIVATE_KEY")
            .map_err(|_| ConfigError::MissingEnvVar("FEE_PAYER_PRIVATE_KEY"))?;

        let universalsettle = if let Ok(program_id_str) =
            std::env::var("UNIVERSALSETTLE_PROGRAM_ID")
        {
            let program_id = Pubkey::from_str(&program_id_str).map_err(|e| {
                ConfigError::InvalidPubkey("UNIVERSALSETTLE_PROGRAM_ID".to_string(), e.to_string())
            })?;
            Some(UniversalSettleConfig {
                program_id,
                onchain: Arc::new(OnceLock::new()),
            })
        } else {
            None
        };

        // Optional: SLAEscrow configuration
        let escrow = if let Ok(program_id_str) = std::env::var("ESCROW_PROGRAM_ID") {
            let program_id = Pubkey::from_str(&program_id_str).map_err(|e| {
                ConfigError::InvalidPubkey("ESCROW_PROGRAM_ID".to_string(), e.to_string())
            })?;

            // Trusted oracle authorities — comma-separated pubkeys.
            //
            // Resolution: `parameters` row [`PR402_ORACLE_AUTHORITIES`]
            // (preferred — survives the Vercel env-var size limit as the
            // operator's oracle ecosystem grows) → legacy `ORACLE_AUTHORITIES`
            // env var → empty list. The DB cache was already warmed by
            // `refresh_parameters_from_db` in `bin/facilitator.rs` before
            // this function ran, so `resolve_string_sync` returns DB values
            // when present.
            let oracle_authorities_raw = parameters::resolve_string_sync(
                parameters::PR402_ORACLE_AUTHORITIES,
                "ORACLE_AUTHORITIES",
            )
            .unwrap_or_default();
            let oracle_authorities = parse_oracle_authorities(&oracle_authorities_raw);

            Some(SLAEscrowConfig {
                program_id,
                bank_address: None,
                fee_bps: None,
                oracle_fee_bps: None,
                oracle_authorities,
            })
        } else {
            None
        };

        let onboard_challenge_ttl_sec = std::env::var("PR402_ONBOARD_CHALLENGE_TTL_SEC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(600);

        let sla_escrow_allow_facilitator_fee_sponsorship =
            env_truthy("PR402_SLA_ESCROW_ALLOW_FACILITATOR_FEE_SPONSORSHIP");

        Ok(Config {
            solana_rpc_url,
            solana_pubsub_url,
            chain_id,
            fee_payer_private_key,
            universalsettle,
            escrow,
            sla_escrow_allow_facilitator_fee_sponsorship,
            onboard_challenge_ttl_sec,
        })
    }
}

/// Env var is true when set to `1`, `true`, or `yes` (ASCII case-insensitive). Unset or other values → false.
fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

impl UniversalSettleConfig {
    /// On-chain params if already loaded (`None` until the first successful [`Self::ensure_loaded`]).
    pub fn onchain(&self) -> Option<UsOnchainParams> {
        self.onchain.get().copied()
    }

    pub fn fee_destination(&self) -> Option<Pubkey> {
        self.onchain().map(|p| p.fee_destination)
    }

    pub fn fee_bps(&self) -> Option<u16> {
        self.onchain().map(|p| p.fee_bps)
    }

    pub fn min_fee_amount(&self) -> Option<u64> {
        self.onchain().map(|p| p.min_fee_amount)
    }

    pub fn min_fee_amount_sol(&self) -> Option<u64> {
        self.onchain().map(|p| p.min_fee_amount_sol)
    }

    /// Lazily load the on-chain Config params, caching the first success. On failure nothing is
    /// cached, so the next call retries (self-healing after a cold-start RPC blip). Idempotent and
    /// safe to call on every hot-path request.
    pub async fn ensure_loaded(
        &self,
        rpc_client: &solana_client::nonblocking::rpc_client::RpcClient,
    ) -> Result<UsOnchainParams, ConfigError> {
        if let Some(params) = self.onchain.get() {
            return Ok(*params);
        }

        let params = self.fetch_onchain(rpc_client).await?;
        // Ignore a lost race: a concurrent caller may have populated the cell first with the
        // same on-chain data. Either way the cell now holds authoritative params.
        let _ = self.onchain.set(params);
        Ok(params)
    }

    /// Read + parse the on-chain Config PDA (no caching).
    async fn fetch_onchain(
        &self,
        rpc_client: &solana_client::nonblocking::rpc_client::RpcClient,
    ) -> Result<UsOnchainParams, ConfigError> {
        // Derive Config PDA
        let (config_pda, _) = Pubkey::find_program_address(&[b"config"], &self.program_id);

        // Read Config account
        let account = rpc_client.get_account(&config_pda).await.map_err(|e| {
            ConfigError::AccountReadFailure(
                "UNIVERSALSETTLE_CONFIG".to_string(),
                format!("Failed to read Config account: {}", e),
            )
        })?;

        // Deserialize Config using bytemuck (skipping 8-byte discriminator)
        let config_size = std::mem::size_of::<USConfig>();
        if account.data.len() < 8 + config_size {
            return Err(ConfigError::AccountReadFailure(
                "UNIVERSALSETTLE_CONFIG".to_string(),
                format!(
                    "Config account data too short (need {} bytes, got {})",
                    8 + config_size,
                    account.data.len()
                ),
            ));
        }

        let config_state = bytemuck::from_bytes::<USConfig>(&account.data[8..8 + config_size]);

        Ok(UsOnchainParams {
            fee_destination: Pubkey::from(config_state.fee_destination.to_bytes()),
            fee_bps: config_state.fee_bps,
            min_fee_amount: config_state.min_fee_amount,
            min_fee_amount_sol: config_state.min_fee_amount_sol,
        })
    }
}

impl SLAEscrowConfig {
    /// Read fee settings from on-chain Bank account.
    ///
    /// The Bank carries a facilitator-wide `fee_bps` only. The oracle tip (`oracle_fee_bps`)
    /// is a **per-escrow** field on the sla-escrow protocol — there is no Bank-level default.
    /// pr402 therefore resolves its default oracle tip from its own operator config
    /// (`PR402_SLA_ESCROW_DEFAULT_ORACLE_FEE_BPS` via `parameters` / env), clamped to
    /// `MAX_ORACLE_FEE_BPS` so misconfiguration can't advertise a value the program rejects.
    pub async fn load_fee_settings(
        &mut self,
        rpc_client: &solana_client::nonblocking::rpc_client::RpcClient,
    ) -> Result<(), ConfigError> {
        // Derive Bank PDA
        let (bank_pda, _) = Pubkey::find_program_address(&[b"bank"], &self.program_id);

        // Read Bank account
        let account = rpc_client.get_account(&bank_pda).await.map_err(|e| {
            ConfigError::AccountReadFailure(
                "SLAESCROW_BANK".to_string(),
                format!("Failed to read Bank account: {}", e),
            )
        })?;

        // Deserialize Bank using bytemuck (skipping 8-byte discriminator)
        let bank_size = std::mem::size_of::<EscrowBank>();
        if account.data.len() < 8 + bank_size {
            return Err(ConfigError::AccountReadFailure(
                "SLAESCROW_BANK".to_string(),
                format!(
                    "Bank account data too short (need {} bytes, got {})",
                    8 + bank_size,
                    account.data.len()
                ),
            ));
        }

        let bank_state = bytemuck::from_bytes::<EscrowBank>(&account.data[8..8 + bank_size]);

        self.fee_bps = Some(bank_state.fee_bps);
        self.bank_address = Some(bank_pda);

        // Operator-configured default oracle tip. Sync resolver: reads the DB parameters
        // cache if warm, else the env var, else `DEFAULT_SLA_ESCROW_ORACLE_FEE_BPS`. The cap
        // matches the on-chain program constant so `/capabilities` advertises a value the
        // program will actually accept.
        self.oracle_fee_bps = Some(crate::parameters::resolve_u16_bps_sync(
            crate::parameters::PR402_SLA_ESCROW_DEFAULT_ORACLE_FEE_BPS,
            crate::parameters::PR402_SLA_ESCROW_DEFAULT_ORACLE_FEE_BPS,
            crate::parameters::DEFAULT_SLA_ESCROW_ORACLE_FEE_BPS,
            sla_escrow_api::consts::MAX_ORACLE_FEE_BPS,
        ));

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Missing required environment variable: {0}")]
    MissingEnvVar(&'static str),
    #[error("Invalid URL for {0}: {1}")]
    InvalidUrl(&'static str, String),
    #[error("Invalid chain ID '{0}': {1}")]
    InvalidChainId(String, String),
    #[error("Invalid pubkey for {0}: {1}")]
    InvalidPubkey(String, String),
    #[error("Account read failure for {0}: {1}")]
    AccountReadFailure(String, String),
}

/// Parse a comma-separated allow-list of base58 oracle pubkeys.
///
/// - Whitespace around each entry is trimmed.
/// - Empty entries are silently dropped (so a trailing comma is harmless).
/// - Entries that fail base58 decode are silently dropped (so a typo in
///   the middle of a long list cannot brick the facilitator at boot).
/// - The output preserves operator-supplied order; pr402's
///   `oracle_authorities.contains(...)` check is order-insensitive but
///   keeping the order makes the published `accepts[].extra.oracleAuthorities`
///   array match the operator's intent.
fn parse_oracle_authorities(raw: &str) -> Vec<Pubkey> {
    raw.split(',')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                Pubkey::from_str(s).ok()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One known-good base58 pubkey from the demo wallets — keeps the
    /// fixture self-contained so test runs don't depend on any external
    /// keypair file.
    const PK_A: &str = "FaciLFwHjbW9V1PtF3vAweL1K1hgin9mvXNXatEQKJdu";
    const PK_B: &str = "oraG62Mr5hDYeSbAtKMpEYFw22SLpZdebXvDe2Qr7xV";

    #[test]
    fn parse_oracle_authorities_empty_returns_empty() {
        assert!(parse_oracle_authorities("").is_empty());
        assert!(parse_oracle_authorities("   ").is_empty());
        assert!(parse_oracle_authorities(",,").is_empty());
    }

    #[test]
    fn parse_oracle_authorities_single_pubkey() {
        let v = parse_oracle_authorities(PK_A);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0], Pubkey::from_str(PK_A).unwrap());
    }

    #[test]
    fn parse_oracle_authorities_multiple_with_whitespace() {
        let v = parse_oracle_authorities(&format!("  {}  , {}  ", PK_A, PK_B));
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], Pubkey::from_str(PK_A).unwrap());
        assert_eq!(v[1], Pubkey::from_str(PK_B).unwrap());
    }

    #[test]
    fn parse_oracle_authorities_drops_invalid_entries() {
        // The middle entry is malformed — a typo in a long allow-list must
        // not bring the facilitator down. Valid entries on either side
        // survive in their declared order.
        let raw = format!("{},not-a-pubkey,{}", PK_A, PK_B);
        let v = parse_oracle_authorities(&raw);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], Pubkey::from_str(PK_A).unwrap());
        assert_eq!(v[1], Pubkey::from_str(PK_B).unwrap());
    }

    #[test]
    fn parse_oracle_authorities_trailing_comma_is_harmless() {
        let v = parse_oracle_authorities(&format!("{},", PK_A));
        assert_eq!(v.len(), 1);
    }
}
