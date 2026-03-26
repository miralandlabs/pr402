//! Simplified configuration for Vercel Serverless environment.
//!
//! Uses environment variables only - no JSON config files needed.

use std::str::FromStr;
use url::Url;

use crate::chain::ChainId;
use solana_pubkey::Pubkey;

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
    /// Fee basis points (read from on-chain Bank account)
    pub fee_bps: Option<u16>,
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
    /// - `PR402_PARAMETERS_CACHE_TTL_SEC`: Revalidate Postgres `parameters` cache (default 60).
    /// - `SOLANA_PUBSUB_URL`: WebSocket URL for pubsub (default: None)
    /// - `MAX_COMPUTE_UNIT_LIMIT`: Max compute units (default: 400000)
    /// - `MAX_COMPUTE_UNIT_PRICE`: Max compute unit price (default: 1000000)
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
            Some(SLAEscrowConfig {
                program_id,
                bank_address: None, // Can be set or derived later
                fee_bps: None,
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

        // Deserialize Config (skip 1-byte discriminator)
        // Config structure: discriminator (1) + authority (32) + fee_destination (32) + updated_at (8) + fee_bps (2) + padding (6)
        if account.data.len() < 1 + 32 + 32 + 8 + 2 + 6 {
            return Err(ConfigError::InvalidChainId(
                "UNIVERSALSETTLE_CONFIG".to_string(),
                "Config account data too short".to_string(),
            ));
        }

        // Extract fee_destination (bytes 33-65: after 1-byte discriminator + 32-byte authority)
        let fee_destination_bytes: [u8; 32] = account.data[33..65].try_into().map_err(|_| {
            ConfigError::InvalidChainId(
                "UNIVERSALSETTLE_CONFIG".to_string(),
                "Failed to extract fee_destination".to_string(),
            )
        })?;

        self.fee_destination = Some(Pubkey::from(fee_destination_bytes));

        // Extract fee_bps (bytes 73-75: after 1 discriminator + 32 authority + 32 fee_dest + 8 updated_at)
        let fee_bps_bytes: [u8; 2] = account.data[73..75].try_into().map_err(|_| {
            ConfigError::InvalidChainId(
                "UNIVERSALSETTLE_CONFIG".to_string(),
                "Failed to extract fee_bps".to_string(),
            )
        })?;
        self.fee_bps = Some(u16::from_le_bytes(fee_bps_bytes));

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
        // Derive Bank PDA (assuming bank 0 for simplicity or configured)
        let (bank_pda, _) = Pubkey::find_program_address(&[b"bank", &[0]], &self.program_id);

        // Read Bank account
        let account = rpc_client.get_account(&bank_pda).await.map_err(|e| {
            ConfigError::InvalidChainId(
                "SLAESCROW_BANK".to_string(),
                format!("Failed to read Bank account: {}", e),
            )
        })?;

        // Deserialize Bank (skip 1-byte discriminator)
        // Bank structure: discriminator (1) + authority (32) + bank_num (1) + fee_bps (2) + ...
        if account.data.len() < 1 + 32 + 1 + 2 {
            return Err(ConfigError::InvalidChainId(
                "SLAESCROW_BANK".to_string(),
                "Bank account data too short".to_string(),
            ));
        }

        // Extract fee_bps (bytes 34-36: after 1 discriminator + 32 authority + 1 bank_num)
        let fee_bps_bytes: [u8; 2] = account.data[34..36].try_into().map_err(|_| {
            ConfigError::InvalidChainId(
                "SLAESCROW_BANK".to_string(),
                "Failed to extract fee_bps from bank".to_string(),
            )
        })?;

        self.fee_bps = Some(u16::from_le_bytes(fee_bps_bytes));
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
