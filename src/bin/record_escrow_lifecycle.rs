//! CLI helper: record SLA escrow post-fund steps into `escrow_lifecycle_events` + `escrow_details`.
//! Used by devnet E2E and operators when HTTP lifecycle routes are not wired yet.

use anyhow::Context;
use clap::{Parser, Subcommand};
use pr402::db::Pr402Db;

#[derive(Parser, Debug)]
#[command(name = "record_escrow_lifecycle")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Record submit-delivery (delivery_hash + delivery_signature).
    SubmitDelivery {
        #[arg(long)]
        correlation_id: String,
        #[arg(long)]
        tx_signature: String,
        #[arg(long)]
        delivery_hash: String,
    },
    /// Record confirm-oracle (resolution_signature, resolution_state, delivery_hash).
    ConfirmOracle {
        #[arg(long)]
        correlation_id: String,
        #[arg(long)]
        tx_signature: String,
        #[arg(long)]
        delivery_hash: String,
        /// 1 = Approved, 2 = Rejected (sla-escrow CLI default 1).
        #[arg(long)]
        resolution_state: i16,
    },
    /// Record release-payment (sets completed_at).
    ReleasePayment {
        #[arg(long)]
        correlation_id: String,
        #[arg(long)]
        tx_signature: String,
    },
    /// Record refund-payment (sets refunded_at).
    RefundPayment {
        #[arg(long)]
        correlation_id: String,
        #[arg(long)]
        tx_signature: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let db_url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .context("DATABASE_URL must be set")?;
    let db = Pr402Db::connect(db_url).map_err(|e| anyhow::anyhow!("{}", e))?;

    match cli.command {
        Commands::SubmitDelivery {
            correlation_id,
            tx_signature,
            delivery_hash,
        } => {
            db.apply_escrow_lifecycle_step(
                &correlation_id,
                "submit_delivery",
                &tx_signature,
                Some(&delivery_hash),
                None,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        }
        Commands::ConfirmOracle {
            correlation_id,
            tx_signature,
            delivery_hash,
            resolution_state,
        } => {
            db.apply_escrow_lifecycle_step(
                &correlation_id,
                "confirm_oracle",
                &tx_signature,
                Some(&delivery_hash),
                Some(resolution_state),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        }
        Commands::ReleasePayment {
            correlation_id,
            tx_signature,
        } => {
            db.apply_escrow_lifecycle_step(
                &correlation_id,
                "release_payment",
                &tx_signature,
                None,
                None,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        }
        Commands::RefundPayment {
            correlation_id,
            tx_signature,
        } => {
            db.apply_escrow_lifecycle_step(
                &correlation_id,
                "refund_payment",
                &tx_signature,
                None,
                None,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        }
    }

    Ok(())
}
