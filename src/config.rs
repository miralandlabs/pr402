//! Simplified configuration for Vercel Serverless environment.
//!
//! Uses environment variables only - no JSON config files needed.

use std::str::FromStr;
use url::Url;

use crate::chain::ChainId;
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
    /// Maximum compute unit limit for transactions
    pub max_compute_unit_limit: u32,
    /// Maximum compute unit price for transactions
    pub max_compute_unit_price: u64,
    /// UniversalSettle configuration (optional)
    pub universalsettle: Option<UniversalSettleConfig>,
    /// SLAEscrow configuration (optional)
    pub escrow: Option<SLAEscrowConfig>,
    /// Challenge validity window (seconds). Default 600; max 3600 enforced in handlers. DB override: `parameters` / `PR402_ONBOARD_CHALLENGE_TTL_SEC`.
    pub onboard_challenge_ttl_sec: u64,
}

/// UniversalSettle configuration for fee-charging facilitator.
#[derive(Debug, Clone)]
pub struct UniversalSettleConfig {
    /// UniversalSettle program ID
    pub program_id: Pubkey,
    /// Fee destination (read from on-chain Config account)
    pub fee_destination: Option<Pubkey>,
    /// Fee basis points (read from on-chain Config account)
    pub fee_bps: Option<u16>,
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
    /// - `MAX_COMPUTE_UNIT_LIMIT`: Max compute units (default: 400000)
    /// - `MAX_COMPUTE_UNIT_PRICE`: Max compute unit price (default: 1000000)
    /// - `UNIVERSALSETTLE_PROGRAM_ID`: Enables `v2:solana:exact` with UniversalSettle (vault, fees, sweep). Omit only if you do not serve that scheme.
    /// - `ESCROW_PROGRAM_ID`: Registers `v2:solana:sla-escrow`. Omit if you do not serve escrow. At least one settlement program should match what RPs advertise.
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

        let max_compute_unit_limit = std::env::var("MAX_COMPUTE_UNIT_LIMIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(400_000);

        let max_compute_unit_price = std::env::var("MAX_COMPUTE_UNIT_PRICE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1_000_000);

        let universalsettle = if let Ok(program_id_str) =
            std::env::var("UNIVERSALSETTLE_PROGRAM_ID")
        {
            let program_id = Pubkey::from_str(&program_id_str).map_err(|e| {
                ConfigError::InvalidChainId("UNIVERSALSETTLE_PROGRAM_ID".to_string(), e.to_string())
            })?;
            Some(UniversalSettleConfig {
                program_id,
                fee_destination: None, // Will be read from chain
                fee_bps: None,         // Will be read from chain
            })
        } else {
            None
        };

        // Optional: SLAEscrow configuration
        let escrow = if let Ok(program_id_str) = std::env::var("ESCROW_PROGRAM_ID") {
            let program_id = Pubkey::from_str(&program_id_str).map_err(|e| {
                ConfigError::InvalidChainId("ESCROW_PROGRAM_ID".to_string(), e.to_string())
            })?;

            // Oracle candidate list (comma-separated pubkeys)
            let oracle_authorities = std::env::var("ORACLE_AUTHORITIES")
                .unwrap_or_default()
                .split(',')
                .filter_map(|s| {
                    let s = s.trim();
                    if s.is_empty() {
                        None
                    } else {
                        Pubkey::from_str(s).ok()
                    }
                })
                .collect::<Vec<Pubkey>>();

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

        Ok(Config {
            solana_rpc_url,
            solana_pubsub_url,
            chain_id,
            fee_payer_private_key,
            max_compute_unit_limit,
            max_compute_unit_price,
            universalsettle,
            escrow,
            onboard_challenge_ttl_sec,
        })
    }
}

impl UniversalSettleConfig {
    /// Read fee destination from on-chain Config account.
    pub async fn load_fee_destination(
        &mut self,
        rpc_client: &solana_client::nonblocking::rpc_client::RpcClient,
    ) -> Result<(), ConfigError> {
        // Derive Config PDA
        let (config_pda, _) = Pubkey::find_program_address(&[b"config"], &self.program_id);

        // Read Config account
        let account = rpc_client.get_account(&config_pda).await.map_err(|e| {
            ConfigError::InvalidChainId(
                "UNIVERSALSETTLE_CONFIG".to_string(),
                format!("Failed to read Config account: {}", e),
            )
        })?;

        // Deserialize Config using bytemuck (skipping 8-byte discriminator)
        if account.data.len() < 8 {
            return Err(ConfigError::InvalidChainId(
                "UNIVERSALSETTLE_CONFIG".to_string(),
                "Config account data too short".to_string(),
            ));
        }

        let config_state =
            bytemuck::try_from_bytes::<USConfig>(&account.data[8..]).map_err(|e| {
                ConfigError::InvalidChainId(
                    "UNIVERSALSETTLE_CONFIG".to_string(),
                    format!("Failed to deserialize UniversalSettle Config: {}", e),
                )
            })?;

        self.fee_destination = Some(Pubkey::from(config_state.fee_destination.to_bytes()));
        self.fee_bps = Some(config_state.fee_bps);

        Ok(())
    }

    pub fn fee_bps(&self) -> Option<u16> {
        self.fee_bps
    }
}

impl SLAEscrowConfig {
    /// Read fee settings from on-chain Bank account.
    pub async fn load_fee_settings(
        &mut self,
        rpc_client: &solana_client::nonblocking::rpc_client::RpcClient,
    ) -> Result<(), ConfigError> {
        // Derive Bank PDA
        let (bank_pda, _) = Pubkey::find_program_address(&[b"bank"], &self.program_id);

        // Read Bank account
        let account = rpc_client.get_account(&bank_pda).await.map_err(|e| {
            ConfigError::InvalidChainId(
                "SLAESCROW_BANK".to_string(),
                format!("Failed to read Bank account: {}", e),
            )
        })?;

        // Deserialize Bank using bytemuck (skipping 8-byte discriminator)
        if account.data.len() < 8 {
            return Err(ConfigError::InvalidChainId(
                "SLAESCROW_BANK".to_string(),
                "Bank account data too short".to_string(),
            ));
        }

        let bank_state =
            bytemuck::try_from_bytes::<EscrowBank>(&account.data[8..]).map_err(|e| {
                ConfigError::InvalidChainId(
                    "SLAESCROW_BANK".to_string(),
                    format!("Failed to deserialize SLAEscrow Bank: {}", e),
                )
            })?;

        self.fee_bps = Some(bank_state.fee_bps);
        self.bank_address = Some(bank_pda);

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
}
