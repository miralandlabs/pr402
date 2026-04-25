//! Chain abstraction for Solana.

mod chain_id;
pub mod solana;
pub mod solana_sla_escrow;
pub mod solana_universalsettle;

pub use chain_id::*;

use crate::config::Config;
use std::str::FromStr;
use std::sync::Arc;

/// Chain provider operations trait.
pub trait ChainProviderOps {
    fn signer_addresses(&self) -> Vec<String>;
    fn chain_id(&self) -> ChainId;
}

/// Solana chain provider wrapper.
#[derive(Clone)]
pub struct ChainProvider {
    pub solana: Arc<solana::SolanaChainProvider>,
    /// HTTP build only: when false, `facilitatorPaysTransactionFees: true` is rejected (see `Config`).
    pub sla_escrow_allow_facilitator_fee_sponsorship: bool,
}

impl ChainProvider {
    /// Create a chain provider from configuration.
    pub async fn from_config(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        use solana_keypair::Keypair;

        let keypair = Keypair::from_base58_string(&config.fee_payer_private_key);

        let _chain_ref = solana::SolanaChainReference::from_str(&config.chain_id.reference)?;

        let _pubsub_url = config.solana_pubsub_url.as_ref().map(|u| u.to_string());

        // Load UniversalSettle config if present
        let mut universalsettle = config.universalsettle.clone();
        let rpc_client = solana_client::nonblocking::rpc_client::RpcClient::new_with_commitment(
            config.solana_rpc_url.as_ref().to_owned(),
            solana_commitment_config::CommitmentConfig::confirmed(),
        );

        if let Some(ref mut us_config) = universalsettle {
            if let Err(e) = us_config.load_fee_destination(&rpc_client).await {
                tracing::warn!(error = %e, "Failed to load UniversalSettle fee destination");
            }
        }

        // Load SLAEscrow config if present
        let mut escrow = config.escrow.clone();
        if let Some(ref mut esc_config) = escrow {
            if let Err(e) = esc_config.load_fee_settings(&rpc_client).await {
                tracing::warn!(error = %e, "Failed to load SLAEscrow bank settings");
            }
        }

        let chain_id = config.chain_id.clone();

        let provider = solana::SolanaChainProvider::new(
            config.solana_rpc_url.as_ref(),
            keypair,
            chain_id.clone(),
            universalsettle,
            escrow,
            config.max_compute_unit_limit,
            config.max_compute_unit_price,
        );

        Ok(ChainProvider {
            solana: Arc::new(provider),
            sla_escrow_allow_facilitator_fee_sponsorship: config
                .sla_escrow_allow_facilitator_fee_sponsorship,
        })
    }
}

impl ChainProviderOps for ChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        self.solana
            .signer_addresses()
            .iter()
            .map(|a| a.to_string())
            .collect()
    }

    fn chain_id(&self) -> ChainId {
        self.solana.chain_id()
    }
}
