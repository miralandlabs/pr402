//! PostgreSQL persistence: **mirror** signer-payer-serverless-copy
//! `signer-payer/src/database.rs` (`Database::new`) for the pool + TLS.
//!
//! **CRUD:** each operation uses the same transaction pattern as `database/base.rs`
//! (`get_platform_parameters`) and `database/payment.rs` (`add_payment`): `BEGIN` →
//! `DEALLOCATE ALL` with `DEALLOCATE_TIMEOUT` and `server_log` tracing → main statement under
//! `QUERY_TIMEOUT` → `COMMIT` or explicit `ROLLBACK` with failure logs.

use deadpool_postgres::{Client, Config, Pool, PoolConfig, Runtime};
use openssl::ssl::{SslConnector, SslMethod};
use postgres_openssl::MakeTlsConnector;
use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;
use tokio::time::timeout;
use tokio_postgres::types::Json;
use tokio_postgres::Transaction;
use tracing::{error, info};

/// Deadpool/tokio-postgres often surface a useless Display ("db error"); walk `source()` for the real message.
fn format_err_chain(err: &dyn Error) -> String {
    let mut out = err.to_string();
    let mut src = err.source();
    while let Some(s) = src {
        out.push_str(" | ");
        out.push_str(&s.to_string());
        src = s.source();
    }
    out
}

/// pr402 facilitator DB pool (deadpool + TLS; parity with signer-payer `Database`).
#[derive(Clone)]
pub struct Pr402Db {
    pool: Pool,
}

#[derive(Debug)]
pub enum DbError {
    Pool(String),
    Query(String),
    Timeout,
    /// Mirror signer-payer `DatabaseError::TransactionFailed`.
    TransactionFailed,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Pool(s) => write!(f, "db pool: {}", s),
            DbError::Query(s) => write!(f, "db query: {}", s),
            DbError::Timeout => write!(f, "db query timed out"),
            DbError::TransactionFailed => write!(f, "database transaction failed"),
        }
    }
}

impl std::error::Error for DbError {}

/// Pairs `resource_providers.settlement_mode` with optional `spl_mint` (keeps call arity down for clippy).
#[derive(Clone, Copy)]
pub struct ResourceProviderRail<'a> {
    pub settlement_mode: &'a str,
    pub spl_mint: Option<&'a str>,
}

/// Enriched x402 V2 metadata for auditing.
#[derive(Default)]
pub struct PaymentAuditMetadata<'a> {
    pub payer_wallet: Option<&'a str>,
    pub scheme: Option<&'a str>,
    pub amount: Option<&'a str>,
    pub asset: Option<&'a str>,
}

/// Outcome of a payment step for auditing.
pub struct PaymentOutcome<'a> {
    pub ok: bool,
    pub error: Option<&'a str>,
    pub signature: Option<&'a str>,
}

/// Identifies the resource provider and their settlement configuration.
pub struct ResourceProviderInfo<'a> {
    pub wallet_pubkey: &'a str,
    pub rail: ResourceProviderRail<'a>,
}

impl Pr402Db {
    const WAIT: Duration = Duration::from_secs(15);
    const CREATE: Duration = Duration::from_secs(10);
    const RECYCLE: Duration = Duration::from_secs(30);
    /// `signer-payer-serverless-copy/signer-payer/src/database/base.rs` + `payment.rs`.
    const DEALLOCATE_TIMEOUT: Duration = Duration::from_secs(5);
    /// `signer-payer` `database.rs`: 60s for Vercel serverless + pooled Postgres.
    const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

    /// Same `match` as signer-payer `get_platform_parameters` / `add_payment` after `BEGIN`.
    async fn deallocate_all_signer_style(tx: &Transaction<'_>) {
        match timeout(Self::DEALLOCATE_TIMEOUT, tx.execute("DEALLOCATE ALL", &[])).await {
            Ok(Ok(_)) => info!(target: "server_log", "DEALLOCATE ALL succeeded"),
            Ok(Err(e)) => error!(target: "server_log", error = %e, "DEALLOCATE ALL failed"),
            Err(_) => error!(
                target: "server_log",
                "DEALLOCATE ALL timed out after {:?}",
                Self::DEALLOCATE_TIMEOUT
            ),
        }
    }

    pub fn connect(database_url: impl Into<String>) -> Result<Self, DbError> {
        let mut cfg = Config::new();
        cfg.url = Some(database_url.into());
        cfg.pool = Some(PoolConfig {
            max_size: 5,
            timeouts: deadpool_postgres::Timeouts {
                wait: Some(Self::WAIT),
                create: Some(Self::CREATE),
                recycle: Some(Self::RECYCLE),
            },
            ..Default::default()
        });

        let mut builder =
            SslConnector::builder(SslMethod::tls()).map_err(|e| DbError::Query(e.to_string()))?;
        builder.set_verify(openssl::ssl::SslVerifyMode::NONE);
        let tls = MakeTlsConnector::new(builder.build());
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), tls)
            .map_err(|e| DbError::Pool(format_err_chain(&e)))?;
        Ok(Pr402Db { pool })
    }

    /// `None` if `var_name` is unset; `Some(Err)` if URL invalid.
    pub fn from_env_var(var_name: &str) -> Option<Result<Self, DbError>> {
        let Ok(url) = std::env::var(var_name) else {
            return None;
        };
        if url.is_empty() {
            return None;
        }
        Some(Self::connect(url))
    }

    async fn conn(&self) -> Result<Client, DbError> {
        self.pool
            .get()
            .await
            .map_err(|e| DbError::Pool(format_err_chain(&e)))
    }

    /// Load `parameters` rows — transaction + `DEALLOCATE ALL` + timeouts like signer-payer `get_platform_parameters`.
    pub async fn fetch_parameters_map(&self) -> Result<HashMap<String, String>, DbError> {
        const SQL: &str = r#"
                SELECT param_name, param_value
                FROM parameters
                WHERE inactive = false
                  AND (effective_from IS NULL OR effective_from <= NOW())
                  AND (expires_at IS NULL OR expires_at > NOW())
                ORDER BY param_name ASC
                "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(Self::QUERY_TIMEOUT, tx.query(SQL, &[])).await {
            Ok(Ok(rows)) => {
                let map: HashMap<String, String> = rows
                    .into_iter()
                    .map(|row| {
                        let name: String = row.get("param_name");
                        let value: String = row.get("param_value");
                        (name, value)
                    })
                    .collect();
                Ok(map)
            }
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "Query failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => {
                error!(
                    target: "server_log",
                    "Query timed out after {:?}",
                    Self::QUERY_TIMEOUT
                );
                Err(DbError::Timeout)
            }
        };

        match result {
            Ok(map) => {
                tx.commit().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Commit failed");
                    DbError::TransactionFailed
                })?;
                Ok(map)
            }
            Err(e) => {
                tx.rollback().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Rollback failed");
                    DbError::TransactionFailed
                })?;
                Err(e)
            }
        }
    }

    /// Ensure a row exists for `wallet_pubkey`; return `id`.
    async fn ensure_resource_provider(
        &self,
        wallet_pubkey: &str,
        settlement_mode: &str,
        spl_mint: Option<&str>,
    ) -> Result<i64, DbError> {
        const SQL: &str = r#"
                INSERT INTO resource_providers (wallet_pubkey, settlement_mode, spl_mint, last_seen_at)
                VALUES ($1, $2, $3, NOW())
                ON CONFLICT (wallet_pubkey, settlement_mode, spl_mint) DO UPDATE SET
                    last_seen_at = NOW()
                RETURNING id
                "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(
            Self::QUERY_TIMEOUT,
            tx.query_one(SQL, &[&wallet_pubkey, &settlement_mode, &spl_mint]),
        )
        .await
        {
            Ok(Ok(row)) => Ok(row.get::<_, i64>("id")),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "ensure_resource_provider query_one failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => {
                error!(
                    target: "server_log",
                    "Query timed out after {:?}",
                    Self::QUERY_TIMEOUT
                );
                Err(DbError::Timeout)
            }
        };

        match result {
            Ok(id) => {
                tx.commit().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Commit failed");
                    DbError::TransactionFailed
                })?;
                Ok(id)
            }
            Err(e) => {
                tx.rollback().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Rollback failed");
                    DbError::TransactionFailed
                })?;
                Err(e)
            }
        }
    }

    /// Upsert provider and cache UniversalSettle PDAs from onboarding / discovery.
    pub async fn upsert_resource_provider_vaults(
        &self,
        wallet_pubkey: &str,
        settlement_mode: &str,
        spl_mint: Option<&str>,
        split_vault_pda: &str,
        vault_sol_storage_pda: &str,
    ) -> Result<i64, DbError> {
        const SQL: &str = r#"
                INSERT INTO resource_providers (
                    wallet_pubkey, settlement_mode, spl_mint,
                    split_vault_pda, vault_sol_storage_pda, last_seen_at
                )
                VALUES ($1, $2, $3, $4, $5, NOW())
                ON CONFLICT (wallet_pubkey, settlement_mode, spl_mint) DO UPDATE SET
                    split_vault_pda = EXCLUDED.split_vault_pda,
                    vault_sol_storage_pda = EXCLUDED.vault_sol_storage_pda,
                    last_seen_at = NOW()
                RETURNING id
                "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(
            Self::QUERY_TIMEOUT,
            tx.query_one(
                SQL,
                &[
                    &wallet_pubkey,
                    &settlement_mode,
                    &spl_mint,
                    &split_vault_pda,
                    &vault_sol_storage_pda,
                ],
            ),
        )
        .await
        {
            Ok(Ok(row)) => Ok(row.get::<_, i64>("id")),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "upsert_resource_provider_vaults failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => {
                error!(
                    target: "server_log",
                    "Query timed out after {:?}",
                    Self::QUERY_TIMEOUT
                );
                Err(DbError::Timeout)
            }
        };

        match result {
            Ok(id) => {
                tx.commit().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Commit failed");
                    DbError::TransactionFailed
                })?;
                Ok(id)
            }
            Err(e) => {
                tx.rollback().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Rollback failed");
                    DbError::TransactionFailed
                })?;
                Err(e)
            }
        }
    }

    /// Like [`Self::upsert_resource_provider_vaults`], but sets `registration_verified_at` (wallet-signed onboard).
    pub async fn upsert_resource_provider_vaults_verified(
        &self,
        wallet_pubkey: &str,
        settlement_mode: &str,
        spl_mint: Option<&str>,
        split_vault_pda: &str,
        vault_sol_storage_pda: &str,
    ) -> Result<i64, DbError> {
        const SQL: &str = r#"
                INSERT INTO resource_providers (
                    wallet_pubkey, settlement_mode, spl_mint,
                    split_vault_pda, vault_sol_storage_pda, last_seen_at, registration_verified_at
                )
                VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
                ON CONFLICT (wallet_pubkey, settlement_mode, spl_mint) DO UPDATE SET
                    split_vault_pda = EXCLUDED.split_vault_pda,
                    vault_sol_storage_pda = EXCLUDED.vault_sol_storage_pda,
                    last_seen_at = NOW(),
                    registration_verified_at = NOW()
                RETURNING id
                "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(
            Self::QUERY_TIMEOUT,
            tx.query_one(
                SQL,
                &[
                    &wallet_pubkey,
                    &settlement_mode,
                    &spl_mint,
                    &split_vault_pda,
                    &vault_sol_storage_pda,
                ],
            ),
        )
        .await
        {
            Ok(Ok(row)) => Ok(row.get::<_, i64>("id")),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "upsert_resource_provider_vaults_verified failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => {
                error!(
                    target: "server_log",
                    "Query timed out after {:?}",
                    Self::QUERY_TIMEOUT
                );
                Err(DbError::Timeout)
            }
        };

        match result {
            Ok(id) => {
                tx.commit().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Commit failed");
                    DbError::TransactionFailed
                })?;
                Ok(id)
            }
            Err(e) => {
                tx.rollback().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Rollback failed");
                    DbError::TransactionFailed
                })?;
                Err(e)
            }
        }
    }

    /// Record or merge `/verify` outcome for a correlation id with enriched x402 V2 metadata.
    pub async fn record_payment_verify(
        &self,
        correlation_id: &str,
        provider: ResourceProviderInfo<'_>,
        outcome: PaymentOutcome<'_>,
        meta: PaymentAuditMetadata<'_>,
    ) -> Result<(), DbError> {
        let provider_id = self
            .ensure_resource_provider(
                provider.wallet_pubkey,
                provider.rail.settlement_mode,
                provider.rail.spl_mint,
            )
            .await?;

        const SQL: &str = r#"
                INSERT INTO payment_attempts (
                    correlation_id, resource_provider_id,
                    verify_at, verify_ok, verify_error, updated_at,
                    payer_wallet, scheme, amount, asset
                )
                VALUES ($1, $2, NOW(), $3, $4, NOW(), $5, $6, $7, $8)
                ON CONFLICT (correlation_id) DO UPDATE SET
                    resource_provider_id = COALESCE(EXCLUDED.resource_provider_id, payment_attempts.resource_provider_id),
                    verify_at = NOW(),
                    verify_ok = EXCLUDED.verify_ok,
                    verify_error = EXCLUDED.verify_error,
                    updated_at = NOW(),
                    payer_wallet = COALESCE(EXCLUDED.payer_wallet, payment_attempts.payer_wallet),
                    scheme       = COALESCE(EXCLUDED.scheme, payment_attempts.scheme),
                    amount       = COALESCE(EXCLUDED.amount, payment_attempts.amount),
                    asset        = COALESCE(EXCLUDED.asset, payment_attempts.asset)
                "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(
            Self::QUERY_TIMEOUT,
            tx.execute(
                SQL,
                &[
                    &correlation_id,
                    &provider_id,
                    &outcome.ok,
                    &outcome.error,
                    &meta.payer_wallet,
                    &meta.scheme,
                    &meta.amount,
                    &meta.asset,
                ],
            ),
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "record_payment_verify failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => {
                error!(
                    target: "server_log",
                    "Query timed out after {:?}",
                    Self::QUERY_TIMEOUT
                );
                Err(DbError::Timeout)
            }
        };

        match result {
            Ok(()) => {
                tx.commit().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Commit failed");
                    DbError::TransactionFailed
                })?;
                Ok(())
            }
            Err(e) => {
                tx.rollback().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Rollback failed");
                    DbError::TransactionFailed
                })?;
                Err(e)
            }
        }
    }

    /// Record or merge `/settle` outcome with enriched x402 V2 metadata.
    pub async fn record_payment_settle(
        &self,
        correlation_id: &str,
        provider: ResourceProviderInfo<'_>,
        outcome: PaymentOutcome<'_>,
        meta: PaymentAuditMetadata<'_>,
    ) -> Result<(), DbError> {
        let provider_id = self
            .ensure_resource_provider(
                provider.wallet_pubkey,
                provider.rail.settlement_mode,
                provider.rail.spl_mint,
            )
            .await?;

        const SQL: &str = r#"
                INSERT INTO payment_attempts (
                    correlation_id, resource_provider_id,
                    settle_at, settle_ok, settle_error, settlement_signature, updated_at,
                    scheme, amount, asset
                )
                VALUES ($1, $2, NOW(), $3, $4, $5, NOW(), $6, $7, $8)
                ON CONFLICT (correlation_id) DO UPDATE SET
                    resource_provider_id = COALESCE(EXCLUDED.resource_provider_id, payment_attempts.resource_provider_id),
                    settle_at = NOW(),
                    settle_ok = EXCLUDED.settle_ok,
                    settle_error = EXCLUDED.settle_error,
                    settlement_signature = COALESCE(EXCLUDED.settlement_signature, payment_attempts.settlement_signature),
                    updated_at = NOW(),
                    scheme = COALESCE(EXCLUDED.scheme, payment_attempts.scheme),
                    amount = COALESCE(EXCLUDED.amount, payment_attempts.amount),
                    asset = COALESCE(EXCLUDED.asset, payment_attempts.asset)
                "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(
            Self::QUERY_TIMEOUT,
            tx.execute(
                SQL,
                &[
                    &correlation_id,
                    &provider_id,
                    &outcome.ok,
                    &outcome.error,
                    &outcome.signature,
                    &meta.scheme,
                    &meta.amount,
                    &meta.asset,
                ],
            ),
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "record_payment_settle failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => {
                error!(
                    target: "server_log",
                    "Query timed out after {:?}",
                    Self::QUERY_TIMEOUT
                );
                Err(DbError::Timeout)
            }
        };

        match result {
            Ok(()) => {
                tx.commit().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Commit failed");
                    DbError::TransactionFailed
                })?;
                Ok(())
            }
            Err(e) => {
                tx.rollback().await.map_err(|e| {
                    error!(target: "server_log", error = %e, "Rollback failed");
                    DbError::TransactionFailed
                })?;
                Err(e)
            }
        }
    }

    /// Record or update specialized SLAEscrow state for one **payment attempt**.
    ///
    /// Matches `escrow_details.escrow_details_one_row_per_payment_attempt`: upsert conflict target is
    /// **`payment_attempt_id`** (not `escrow_pda`; many payments share the same escrow PDA).
    pub async fn upsert_escrow_detail(
        &self,
        correlation_id: &str,
        escrow_pda: &str,
        bank_pda: &str,
        oracle_authority: &str,
        sla_hash: Option<&str>,
        fund_signature: Option<&str>,
    ) -> Result<(), DbError> {
        const SELECT_ID: &str = r#"SELECT id FROM payment_attempts WHERE correlation_id = $1"#;
        const SQL: &str = r#"
                INSERT INTO escrow_details (
                    payment_attempt_id, escrow_pda, bank_pda, oracle_authority, sla_hash, fund_signature, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, NOW())
                ON CONFLICT (payment_attempt_id) DO UPDATE SET
                    escrow_pda = EXCLUDED.escrow_pda,
                    bank_pda = EXCLUDED.bank_pda,
                    oracle_authority = EXCLUDED.oracle_authority,
                    sla_hash = COALESCE(EXCLUDED.sla_hash, escrow_details.sla_hash),
                    fund_signature = COALESCE(EXCLUDED.fund_signature, escrow_details.fund_signature),
                    updated_at = NOW()
                "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        // 1. Resolve payment_attempt_id
        let attempt_id = match timeout(
            Self::QUERY_TIMEOUT,
            tx.query_opt(SELECT_ID, &[&correlation_id]),
        )
        .await
        {
            Ok(Ok(Some(row))) => row.get::<_, i64>("id"),
            Ok(Ok(None)) => {
                error!(target: "server_log", "Parent payment attempt not found for correlation_id: {}", correlation_id);
                tx.rollback().await.ok();
                return Err(DbError::Query(
                    "Parent payment attempt not found".to_string(),
                ));
            }
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "payment_attempts id lookup failed (escrow upsert)");
                tx.rollback().await.ok();
                return Err(DbError::Query(format_err_chain(&e)));
            }
            Err(_) => {
                error!(
                    target: "server_log",
                    "payment_attempts id lookup timed out after {:?}",
                    Self::QUERY_TIMEOUT
                );
                tx.rollback().await.ok();
                return Err(DbError::Timeout);
            }
        };

        // 2. Upsert detail
        let result = match timeout(
            Self::QUERY_TIMEOUT,
            tx.execute(
                SQL,
                &[
                    &attempt_id,
                    &escrow_pda,
                    &bank_pda,
                    &oracle_authority,
                    &sla_hash,
                    &fund_signature,
                ],
            ),
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "upsert_escrow_detail execute failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => {
                error!(
                    target: "server_log",
                    "upsert_escrow_detail execute timed out after {:?}",
                    Self::QUERY_TIMEOUT
                );
                Err(DbError::Timeout)
            }
        };

        match result {
            Ok(()) => {
                tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
                Ok(())
            }
            Err(e) => {
                tx.rollback().await.ok();
                Err(e)
            }
        }
    }

    fn normalize_escrow_lifecycle_step(step: &str) -> Option<&'static str> {
        match step {
            "submit_delivery" | "submit-delivery" => Some("submit_delivery"),
            "confirm_oracle" | "confirm-oracle" => Some("confirm_oracle"),
            "release_payment" | "release-payment" => Some("release_payment"),
            "refund_payment" | "refund-payment" => Some("refund_payment"),
            _ => None,
        }
    }

    /// Append one `escrow_lifecycle_events` row and update `escrow_details` for that payment attempt (single transaction).
    ///
    /// `step`: `submit_delivery`, `confirm_oracle`, `release_payment`, or `refund_payment` (hyphen forms accepted).
    /// Matches sla-escrow CLI resolution: **1** = Approved, **2** = Rejected.
    pub async fn apply_escrow_lifecycle_step(
        &self,
        correlation_id: &str,
        step: &str,
        tx_signature: &str,
        delivery_hash_hex: Option<&str>,
        resolution_state: Option<i16>,
    ) -> Result<(), DbError> {
        let Some(step_norm) = Self::normalize_escrow_lifecycle_step(step) else {
            return Err(DbError::Query(format!(
                "unknown escrow lifecycle step: {}",
                step
            )));
        };

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        const SELECT_ID: &str = r#"SELECT id FROM payment_attempts WHERE correlation_id = $1"#;
        let attempt_id = match timeout(
            Self::QUERY_TIMEOUT,
            tx.query_opt(SELECT_ID, &[&correlation_id]),
        )
        .await
        {
            Ok(Ok(Some(row))) => row.get::<_, i64>("id"),
            Ok(Ok(None)) => {
                error!(
                    target: "server_log",
                    correlation_id = %correlation_id,
                    "payment_attempts row missing for escrow lifecycle"
                );
                tx.rollback().await.ok();
                return Err(DbError::Query("payment_attempts row not found".into()));
            }
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "lifecycle payment_attempts lookup failed");
                tx.rollback().await.ok();
                return Err(DbError::Query(format_err_chain(&e)));
            }
            Err(_) => {
                tx.rollback().await.ok();
                return Err(DbError::Timeout);
            }
        };

        let require_hash = matches!(step_norm, "submit_delivery" | "confirm_oracle");
        if require_hash && delivery_hash_hex.is_none() {
            tx.rollback().await.ok();
            return Err(DbError::Query(format!(
                "step {} requires delivery_hash_hex",
                step_norm
            )));
        }
        if matches!(step_norm, "confirm_oracle") && resolution_state.is_none() {
            tx.rollback().await.ok();
            return Err(DbError::Query(format!(
                "step {} requires resolution_state (1 approved, 2 rejected)",
                step_norm
            )));
        }
        let h = delivery_hash_hex.unwrap_or("");
        if require_hash && (h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit())) {
            tx.rollback().await.ok();
            return Err(DbError::Query(
                "delivery_hash_hex must be 64 hex characters".into(),
            ));
        }

        let payload_value = match step_norm {
            "submit_delivery" => json!({ "delivery_hash": h }),
            "confirm_oracle" => json!({
                "delivery_hash": h,
                "resolution_state": resolution_state.unwrap(),
            }),
            "release_payment" => json!({}),
            "refund_payment" => json!({}),
            _ => json!({}),
        };
        const INSERT_EV: &str = r#"
            INSERT INTO escrow_lifecycle_events (payment_attempt_id, step, tx_signature, payload)
            VALUES ($1, $2, $3, $4)
            "#;

        let payload_json = Json(payload_value);

        let ins = match timeout(
            Self::QUERY_TIMEOUT,
            tx.execute(
                INSERT_EV,
                &[&attempt_id, &step_norm, &tx_signature, &payload_json],
            ),
        )
        .await
        {
            Ok(Ok(n)) if n > 0 => Ok(()),
            Ok(Ok(_)) => Err(DbError::Query(
                "escrow_lifecycle_events insert failed".into(),
            )),
            Ok(Err(e)) => Err(DbError::Query(format_err_chain(&e))),
            Err(_) => Err(DbError::Timeout),
        };
        if let Err(e) = ins {
            tx.rollback().await.ok();
            error!(
                target: "server_log",
                correlation_id = %correlation_id,
                step = %step_norm,
                error = %e,
                "escrow lifecycle event insert failed"
            );
            return Err(e);
        }

        let upd = match step_norm {
            "submit_delivery" => {
                const SQL: &str = r#"
                    UPDATE escrow_details
                    SET delivery_hash = $2,
                        delivery_signature = $3,
                        updated_at = NOW()
                    WHERE payment_attempt_id = $1
                    "#;
                timeout(
                    Self::QUERY_TIMEOUT,
                    tx.execute(SQL, &[&attempt_id, &h, &tx_signature]),
                )
                .await
            }
            "confirm_oracle" => {
                let rs = resolution_state.unwrap();
                const SQL: &str = r#"
                    UPDATE escrow_details
                    SET delivery_hash = $2,
                        resolution_signature = $3,
                        resolution_state = $4,
                        updated_at = NOW()
                    WHERE payment_attempt_id = $1
                    "#;
                timeout(
                    Self::QUERY_TIMEOUT,
                    tx.execute(SQL, &[&attempt_id, &h, &tx_signature, &rs]),
                )
                .await
            }
            "release_payment" => {
                const SQL: &str = r#"
                    UPDATE escrow_details
                    SET completed_at = NOW(),
                        updated_at = NOW()
                    WHERE payment_attempt_id = $1
                    "#;
                timeout(Self::QUERY_TIMEOUT, tx.execute(SQL, &[&attempt_id])).await
            }
            "refund_payment" => {
                const SQL: &str = r#"
                    UPDATE escrow_details
                    SET refunded_at = NOW(),
                        updated_at = NOW()
                    WHERE payment_attempt_id = $1
                    "#;
                timeout(Self::QUERY_TIMEOUT, tx.execute(SQL, &[&attempt_id])).await
            }
            _ => unreachable!(),
        };

        match upd {
            Ok(Ok(n)) if n > 0 => {}
            Ok(Ok(_)) => {
                tx.rollback().await.ok();
                error!(
                    target: "server_log",
                    correlation_id = %correlation_id,
                    step = %step_norm,
                    "escrow_details lifecycle update matched no row (fund/verify may not have created escrow_details)"
                );
                return Err(DbError::Query(
                    "escrow_details row not found for this payment_attempt".into(),
                ));
            }
            Ok(Err(e)) => {
                tx.rollback().await.ok();
                error!(
                    target: "server_log",
                    correlation_id = %correlation_id,
                    error = %format_err_chain(&e),
                    "escrow_details lifecycle update failed"
                );
                return Err(DbError::Query(format_err_chain(&e)));
            }
            Err(_) => {
                tx.rollback().await.ok();
                return Err(DbError::Timeout);
            }
        }

        tx.commit().await.map_err(|e| {
            error!(target: "server_log", error = %e, "apply_escrow_lifecycle_step commit failed");
            DbError::TransactionFailed
        })?;

        info!(
            target: "server_log",
            correlation_id = %correlation_id,
            step = %step_norm,
            tx_signature = %tx_signature,
            "escrow lifecycle step recorded"
        );

        Ok(())
    }
}
