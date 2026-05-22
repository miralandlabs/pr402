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

/// One row emitted by [`Pr402Db::list_public_providers`] / [`Pr402Db::get_public_provider`].
///
/// Intentionally minimal: only fields the seller opted to publish, plus the vault PDA so
/// buyers / discovery consumers can skip a separate `/discovery` round-trip when they want
/// to build an `accepts[]` line. Sensitive / internal columns (sweep signatures, attempt
/// counters) are never included.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicProviderEntry {
    pub wallet_pubkey: String,
    pub settlement_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spl_mint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_vault_pda: Option<String>,
    pub service_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_metadata: Option<serde_json::Value>,
    /// When the seller signed the onboard challenge for this row. RFC3339 string so
    /// JSON serialization is straightforward even in clients without date libraries.
    pub registration_verified_at: String,
    /// Last time the row was touched by the facilitator (metadata update, discovery
    /// flag flip, etc.). Reuse as the pagination cursor for the next page.
    pub updated_at: String,
}

impl PublicProviderEntry {
    fn from_row(row: &tokio_postgres::Row) -> Self {
        Self {
            wallet_pubkey: row.get::<_, String>("wallet_pubkey"),
            settlement_mode: row.get::<_, String>("settlement_mode"),
            spl_mint: row.try_get::<_, Option<String>>("spl_mint").ok().flatten(),
            split_vault_pda: row
                .try_get::<_, Option<String>>("split_vault_pda")
                .ok()
                .flatten(),
            service_url: row
                .try_get::<_, Option<String>>("service_url")
                .ok()
                .flatten()
                .unwrap_or_default(),
            display_name: row
                .try_get::<_, Option<String>>("display_name")
                .ok()
                .flatten(),
            description: row
                .try_get::<_, Option<String>>("description")
                .ok()
                .flatten(),
            tags: row
                .try_get::<_, Option<Vec<String>>>("tags")
                .ok()
                .flatten()
                .unwrap_or_default(),
            service_metadata: row
                .try_get::<_, Option<Json<serde_json::Value>>>("service_metadata")
                .ok()
                .flatten()
                .map(|j| j.0),
            registration_verified_at: row
                .try_get::<_, Option<std::time::SystemTime>>("registration_verified_at")
                .ok()
                .flatten()
                .map(system_time_to_rfc3339)
                .unwrap_or_default(),
            updated_at: row
                .try_get::<_, std::time::SystemTime>("updated_at")
                .map(system_time_to_rfc3339)
                .unwrap_or_default(),
        }
    }
}

/// One row emitted by [`Pr402Db::list_seller_payments`]. Mirrors the `payment_attempts`
/// columns a seller actually needs to reconcile — no verify/settle error prose.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SellerPaymentEntry {
    pub correlation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settle_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settle_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settlement_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payer_wallet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    pub created_at: String,
}

impl SellerPaymentEntry {
    fn from_row(row: &tokio_postgres::Row) -> Self {
        let iso = |c: &str| {
            row.try_get::<_, Option<std::time::SystemTime>>(c)
                .ok()
                .flatten()
                .map(system_time_to_rfc3339)
        };
        Self {
            correlation_id: row.get::<_, String>("correlation_id"),
            verify_at: iso("verify_at"),
            verify_ok: row.try_get::<_, Option<bool>>("verify_ok").ok().flatten(),
            settle_at: iso("settle_at"),
            settle_ok: row.try_get::<_, Option<bool>>("settle_ok").ok().flatten(),
            settlement_signature: row
                .try_get::<_, Option<String>>("settlement_signature")
                .ok()
                .flatten(),
            payer_wallet: row
                .try_get::<_, Option<String>>("payer_wallet")
                .ok()
                .flatten(),
            scheme: row.try_get::<_, Option<String>>("scheme").ok().flatten(),
            amount: row.try_get::<_, Option<String>>("amount").ok().flatten(),
            asset: row.try_get::<_, Option<String>>("asset").ok().flatten(),
            created_at: row
                .try_get::<_, std::time::SystemTime>("created_at")
                .map(system_time_to_rfc3339)
                .unwrap_or_default(),
        }
    }
}

/// Render a `SystemTime` as RFC3339 / ISO-8601 with microsecond precision. No chrono or
/// time-crate dependency: we compute seconds + fractional seconds from the UNIX_EPOCH and
/// delegate to `humantime_serde`-compatible formatting via manual splitting of UTC components
/// using the `time` crate's primitives... but we don't have `time` either, so we do it from
/// scratch with a verified algorithm. Keeps the database module dep-light.
fn system_time_to_rfc3339(t: std::time::SystemTime) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let dur = t.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let total_secs = dur.as_secs() as i64;
    let micros = dur.subsec_micros();

    // Convert total_secs since 1970-01-01 UTC to (year, month, day, hh, mm, ss).
    // Algorithm from Howard Hinnant "date" (civil_from_days) — public domain / CC0.
    let days = total_secs.div_euclid(86_400);
    let mut secs_of_day = total_secs.rem_euclid(86_400);
    let hh = secs_of_day / 3600;
    secs_of_day %= 3600;
    let mm = secs_of_day / 60;
    let ss = secs_of_day % 60;

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z",
        year, m, d, hh, mm, ss, micros
    )
}

#[derive(Debug)]
pub enum DbError {
    Pool(String),
    Query(String),
    /// Operator / integrator policy (user-facing; no `db query:` prefix).
    FacilitatorPolicy(String),
    Timeout,
    /// Mirror signer-payer `DatabaseError::TransactionFailed`.
    TransactionFailed,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Pool(s) => write!(f, "db pool: {}", s),
            DbError::Query(s) => write!(f, "db query: {}", s),
            DbError::FacilitatorPolicy(s) => write!(f, "{}", s),
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

/// One provider rail candidate for cron-driven sweep.
#[derive(Debug, Clone)]
pub struct SweepCandidate {
    pub wallet_pubkey: String,
    pub settlement_mode: String,
    pub spl_mint: Option<String>,
    pub sweep_threshold: Option<u64>,
}

/// One funded sla-escrow payment that may be eligible for permissionless
/// settlement (`ReleasePayment` / `RefundPayment`) per
/// `oracles/spec/sla-escrow-onchain-abi/v1/NORMATIVE.md` §5.3 / §5.4.
///
/// pr402 reads each candidate's on-chain `Payment` PDA before deciding
/// what action to take; this struct is just the DB selection result.
#[derive(Debug, Clone)]
pub struct SlaEscrowSettleCandidate {
    pub correlation_id: String,
    /// On-chain `Payment.payment_uid` as 64-char lowercase hex.
    pub payment_uid_hex: String,
    pub escrow_pda: String,
    pub bank_pda: String,
    /// Mint pubkey (base58). `None` for native SOL escrows.
    pub mint: Option<String>,
    pub buyer_wallet: Option<String>,
    pub seller_wallet: Option<String>,
    pub oracle_authority: String,
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

    /// Returns `true` when at least one `resource_providers` row exists for this wallet
    /// with `registration_verified_at IS NOT NULL`. Used by the seller lifecycle ladder on
    /// `GET /onboard` to report the `verified` stage without scanning metadata in the UI.
    ///
    /// Scoped to `wallet_pubkey` only (all settlement rails collapse together): a seller is
    /// "verified" once any rail has completed the challenge + signed submit. The facilitator
    /// still enforces the one-asset-per-wallet policy at write time.
    pub async fn resource_provider_verified(&self, wallet_pubkey: &str) -> Result<bool, DbError> {
        const SQL: &str = r#"
            SELECT 1
              FROM resource_providers
             WHERE wallet_pubkey = $1
               AND registration_verified_at IS NOT NULL
               AND (retired_at IS NULL)
             LIMIT 1
        "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "resource_provider_verified: tx start failed");
            DbError::TransactionFailed
        })?;
        Self::deallocate_all_signer_style(&tx).await;

        let res = match timeout(Self::QUERY_TIMEOUT, tx.query_opt(SQL, &[&wallet_pubkey])).await {
            Ok(Ok(row)) => Ok(row.is_some()),
            Ok(Err(e)) => {
                error!(
                    target: "server_log",
                    error = %format_err_chain(&e),
                    "resource_provider_verified query failed"
                );
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => Err(DbError::Timeout),
        };

        if res.is_ok() {
            tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        } else {
            let _ = tx.rollback().await;
        }

        res
    }

    /// Apply an optional discovery payload to the seller's registry rows. Updates *all*
    /// rails for `wallet_pubkey` in lockstep so the public listing stays consistent across
    /// settlement modes. Called at the tail of `POST /onboard`.
    ///
    /// Length / pattern limits are enforced in the application layer (handler) before this
    /// runs; this function just writes the validated values. Passing `None` for any field
    /// leaves the existing column untouched (vs. clearing it), so a seller can update
    /// the display name without wiping tags.
    #[allow(clippy::too_many_arguments)]
    pub async fn apply_seller_discovery(
        &self,
        wallet_pubkey: &str,
        service_url: Option<&str>,
        display_name: Option<&str>,
        description: Option<&str>,
        tags: Option<&[String]>,
        service_metadata: Option<&serde_json::Value>,
        listing_opt_in: Option<bool>,
    ) -> Result<u64, DbError> {
        const SQL: &str = r#"
            UPDATE resource_providers SET
                service_url      = COALESCE($2, service_url),
                display_name     = COALESCE($3, display_name),
                description      = COALESCE($4, description),
                tags             = COALESCE($5, tags),
                service_metadata = COALESCE($6, service_metadata),
                listing_opt_in   = COALESCE($7, listing_opt_in),
                updated_at       = NOW()
             WHERE wallet_pubkey = $1
               AND (retired_at IS NULL)
        "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "apply_seller_discovery: tx start failed");
            DbError::TransactionFailed
        })?;
        Self::deallocate_all_signer_style(&tx).await;

        let metadata_json = service_metadata.map(Json);
        // Owned so references live long enough for the query call.
        let tags_owned: Option<Vec<String>> = tags.map(|t| t.to_vec());

        let res = match timeout(
            Self::QUERY_TIMEOUT,
            tx.execute(
                SQL,
                &[
                    &wallet_pubkey,
                    &service_url,
                    &display_name,
                    &description,
                    &tags_owned,
                    &metadata_json,
                    &listing_opt_in,
                ],
            ),
        )
        .await
        {
            Ok(Ok(n)) => Ok(n),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "apply_seller_discovery failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => Err(DbError::Timeout),
        };

        match res {
            Ok(n) => {
                tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
                Ok(n)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    /// Retire all `resource_providers` rows for a wallet. Sets `retired_at = NOW()` (if not
    /// already set) and flips `inactive = TRUE`. Called from `POST /onboard/retire` after
    /// the same HMAC challenge + wallet signature verification as `POST /onboard`.
    ///
    /// Returns the count of rows updated. Zero = wallet was not in the registry; callers
    /// should surface that as a no-op success, not an error.
    pub async fn retire_resource_provider(&self, wallet_pubkey: &str) -> Result<u64, DbError> {
        const SQL: &str = r#"
            UPDATE resource_providers SET
                retired_at     = COALESCE(retired_at, NOW()),
                inactive       = TRUE,
                listing_opt_in = FALSE,
                updated_at     = NOW()
             WHERE wallet_pubkey = $1
        "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "retire_resource_provider: tx start failed");
            DbError::TransactionFailed
        })?;
        Self::deallocate_all_signer_style(&tx).await;

        let res = match timeout(Self::QUERY_TIMEOUT, tx.execute(SQL, &[&wallet_pubkey])).await {
            Ok(Ok(n)) => Ok(n),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "retire_resource_provider failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => Err(DbError::Timeout),
        };

        match res {
            Ok(n) => {
                tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
                Ok(n)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    /// Refuses to proceed when the wallet has an unretired row that already opted into
    /// verification — a guard against accidentally re-submitting the HMAC challenge for a
    /// wallet that has been retired. Returns `Ok(true)` when any unretired row exists,
    /// `Ok(false)` otherwise.
    pub async fn resource_provider_has_active_row(
        &self,
        wallet_pubkey: &str,
    ) -> Result<bool, DbError> {
        const SQL: &str = r#"
            SELECT 1
              FROM resource_providers
             WHERE wallet_pubkey = $1
               AND (retired_at IS NULL)
             LIMIT 1
        "#;

        let mut client = self.conn().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|_| DbError::TransactionFailed)?;
        Self::deallocate_all_signer_style(&tx).await;

        let res = match timeout(Self::QUERY_TIMEOUT, tx.query_opt(SQL, &[&wallet_pubkey])).await {
            Ok(Ok(row)) => Ok(row.is_some()),
            Ok(Err(e)) => Err(DbError::Query(format_err_chain(&e))),
            Err(_) => Err(DbError::Timeout),
        };

        if res.is_ok() {
            tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        } else {
            let _ = tx.rollback().await;
        }
        res
    }

    /// List public directory entries. Filters are hard-coded to `listing_opt_in = TRUE` +
    /// `registration_verified_at IS NOT NULL` + `inactive = FALSE` + `retired_at IS NULL`,
    /// matching the partial index `idx_resource_providers_public_listing`. Pagination uses a
    /// simple `updated_at < cursor` cursor so the caller can page backwards by passing the
    /// previous page's last `updated_at`.
    ///
    /// `limit` is clamped to `[1, 100]`. Tags are summarized as JSON so the row shape is
    /// stable regardless of the client's array-decoding story.
    pub async fn list_public_providers(
        &self,
        limit: i64,
        cursor_updated_at: Option<std::time::SystemTime>,
    ) -> Result<Vec<PublicProviderEntry>, DbError> {
        let limit = limit.clamp(1, 100);

        let rows = {
            let mut client = self.conn().await?;
            let tx = client
                .transaction()
                .await
                .map_err(|_| DbError::TransactionFailed)?;
            Self::deallocate_all_signer_style(&tx).await;

            const SQL_PAGE: &str = r#"
                SELECT wallet_pubkey,
                       settlement_mode,
                       spl_mint,
                       split_vault_pda,
                       service_url,
                       display_name,
                       description,
                       tags,
                       service_metadata,
                       registration_verified_at,
                       updated_at
                  FROM resource_providers
                 WHERE listing_opt_in = TRUE
                   AND registration_verified_at IS NOT NULL
                   AND inactive = FALSE
                   AND retired_at IS NULL
                   AND ($2::timestamptz IS NULL OR updated_at < $2::timestamptz)
                 ORDER BY updated_at DESC
                 LIMIT $1
            "#;

            let res = timeout(
                Self::QUERY_TIMEOUT,
                tx.query(SQL_PAGE, &[&limit, &cursor_updated_at]),
            )
            .await;

            let rows = match res {
                Ok(Ok(rows)) => rows,
                Ok(Err(e)) => {
                    let _ = tx.rollback().await;
                    return Err(DbError::Query(format_err_chain(&e)));
                }
                Err(_) => {
                    let _ = tx.rollback().await;
                    return Err(DbError::Timeout);
                }
            };

            tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
            rows
        };

        Ok(rows.iter().map(PublicProviderEntry::from_row).collect())
    }

    /// Single-wallet lookup for the public directory. Applies the same visibility filters as
    /// `list_public_providers`. Returns `None` when no public row exists — callers surface that
    /// as HTTP 404.
    pub async fn get_public_provider(
        &self,
        wallet_pubkey: &str,
    ) -> Result<Option<PublicProviderEntry>, DbError> {
        const SQL: &str = r#"
            SELECT wallet_pubkey,
                   settlement_mode,
                   spl_mint,
                   split_vault_pda,
                   service_url,
                   display_name,
                   description,
                   tags,
                   service_metadata,
                   registration_verified_at,
                   updated_at
              FROM resource_providers
             WHERE wallet_pubkey = $1
               AND listing_opt_in = TRUE
               AND registration_verified_at IS NOT NULL
               AND inactive = FALSE
               AND retired_at IS NULL
             ORDER BY updated_at DESC
             LIMIT 1
        "#;

        let mut client = self.conn().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|_| DbError::TransactionFailed)?;
        Self::deallocate_all_signer_style(&tx).await;

        let res = timeout(Self::QUERY_TIMEOUT, tx.query_opt(SQL, &[&wallet_pubkey])).await;
        let row = match res {
            Ok(Ok(Some(row))) => Some(row),
            Ok(Ok(None)) => None,
            Ok(Err(e)) => {
                let _ = tx.rollback().await;
                return Err(DbError::Query(format_err_chain(&e)));
            }
            Err(_) => {
                let _ = tx.rollback().await;
                return Err(DbError::Timeout);
            }
        };
        tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        Ok(row.as_ref().map(PublicProviderEntry::from_row))
    }

    /// List recent settled payments for a seller wallet. Joins through the resource-provider id.
    /// Pagination uses `created_at < cursor`; limit clamped to `[1, 100]`.
    pub async fn list_seller_payments(
        &self,
        wallet_pubkey: &str,
        limit: i64,
        cursor_created_at: Option<std::time::SystemTime>,
    ) -> Result<Vec<SellerPaymentEntry>, DbError> {
        let limit = limit.clamp(1, 100);

        const SQL: &str = r#"
            SELECT pa.correlation_id,
                   pa.verify_at,
                   pa.verify_ok,
                   pa.settle_at,
                   pa.settle_ok,
                   pa.settlement_signature,
                   pa.payer_wallet,
                   pa.scheme,
                   pa.amount,
                   pa.asset,
                   pa.created_at
              FROM payment_attempts pa
              JOIN resource_providers rp ON rp.id = pa.resource_provider_id
             WHERE rp.wallet_pubkey = $1
               AND ($3::timestamptz IS NULL OR pa.created_at < $3::timestamptz)
             ORDER BY pa.created_at DESC
             LIMIT $2
        "#;

        let mut client = self.conn().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|_| DbError::TransactionFailed)?;
        Self::deallocate_all_signer_style(&tx).await;

        let res = timeout(
            Self::QUERY_TIMEOUT,
            tx.query(SQL, &[&wallet_pubkey, &limit, &cursor_created_at]),
        )
        .await;

        let rows = match res {
            Ok(Ok(rows)) => rows,
            Ok(Err(e)) => {
                let _ = tx.rollback().await;
                return Err(DbError::Query(format_err_chain(&e)));
            }
            Err(_) => {
                let _ = tx.rollback().await;
                return Err(DbError::Timeout);
            }
        };
        tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        Ok(rows.iter().map(SellerPaymentEntry::from_row).collect())
    }

    pub async fn ping(&self) -> Result<(), DbError> {
        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Ping transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let res = match timeout(Duration::from_secs(5), tx.execute("SELECT 1", &[])).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "Ping query failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => {
                error!(target: "server_log", "Ping query timed out");
                Err(DbError::Timeout)
            }
        };

        if res.is_ok() {
            tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        } else {
            tx.rollback().await.ok();
        }
        res
    }

    /// One **merchant** `wallet_pubkey` may only use a single settlement rail (`settlement_mode` +
    /// `spl_mint`) while this policy is enabled. Empty DB → OK. Existing row with a different rail →
    /// [`DbError::Query`]. (Index DDL unchanged for future multi-asset loosening.)
    pub async fn assert_merchant_single_rail_policy(
        &self,
        merchant_wallet_pubkey: &str,
        settlement_mode: &str,
        spl_mint: Option<&str>,
    ) -> Result<(), DbError> {
        const SQL: &str = r#"
                SELECT DISTINCT settlement_mode, spl_mint
                FROM resource_providers
                WHERE wallet_pubkey = $1 AND inactive = false
                "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(
            Self::QUERY_TIMEOUT,
            tx.query(SQL, &[&merchant_wallet_pubkey]),
        )
        .await
        {
            Ok(Ok(rows)) => Ok(rows),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "assert_merchant_single_rail_policy query failed");
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

        let rows = match result {
            Ok(r) => r,
            Err(e) => {
                tx.rollback().await.ok();
                return Err(e);
            }
        };

        let mut rails: Vec<(String, Option<String>)> = Vec::new();
        for row in rows {
            let mode: String = row.get("settlement_mode");
            let mint: Option<String> = row.get("spl_mint");
            rails.push((mode, mint));
        }

        tx.commit().await.map_err(|_| DbError::TransactionFailed)?;

        if rails.is_empty() {
            return Ok(());
        }

        // Policy: One Native SOL rail (None) AND at most one SPL rail (Some).
        // Conflict only if adding a different SPL rail when one already exists,
        // or if the settlement_mode is inconsistent.
        let existing_spl = rails.iter().find_map(|(_, m)| m.as_ref());
        let adding_spl = spl_mint;

        if let (Some(existing), Some(adding)) = (existing_spl, adding_spl) {
            if existing != adding {
                return Err(DbError::FacilitatorPolicy(
                    format!("This merchant wallet is already registered with SPL token {}. Use that asset in accepts[], or use a different seller wallet (one SPL asset per wallet policy).", existing)
                ));
            }
        }

        // Also check settlement_mode consistency (usually v2:solana:exact)
        for (mode, _) in &rails {
            if mode != settlement_mode {
                return Err(DbError::FacilitatorPolicy(format!(
                    "Inconsistent settlement mode: existing={}, requested={}",
                    mode, settlement_mode
                )));
            }
        }

        Ok(())
    }

    /// Ensure a row exists for `wallet_pubkey`; return `id`.
    async fn ensure_resource_provider(
        &self,
        wallet_pubkey: &str,
        settlement_mode: &str,
        spl_mint: Option<&str>,
    ) -> Result<i64, DbError> {
        self.assert_merchant_single_rail_policy(wallet_pubkey, settlement_mode, spl_mint)
            .await?;

        const SQL: &str = r#"
                INSERT INTO resource_providers (wallet_pubkey, settlement_mode, spl_mint, updated_at)
                VALUES ($1, $2, $3, NOW())
                ON CONFLICT (wallet_pubkey, settlement_mode, spl_mint) DO UPDATE SET
                    updated_at = NOW()
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
        self.assert_merchant_single_rail_policy(wallet_pubkey, settlement_mode, spl_mint)
            .await?;

        const SQL: &str = r#"
                INSERT INTO resource_providers (
                    wallet_pubkey, settlement_mode, spl_mint,
                    split_vault_pda, vault_sol_storage_pda, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, NOW())
                ON CONFLICT (wallet_pubkey, settlement_mode, spl_mint) DO UPDATE SET
                    split_vault_pda = EXCLUDED.split_vault_pda,
                    vault_sol_storage_pda = EXCLUDED.vault_sol_storage_pda,
                    updated_at = NOW()
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
        self.assert_merchant_single_rail_policy(wallet_pubkey, settlement_mode, spl_mint)
            .await?;

        // NOTE: on conflict we also clear the retirement markers so a seller can
        // reactivate a previously retired wallet by re-running the Activate → Verify
        // ladder. Without this, POST /onboard would return 200 OK but the row would
        // remain `inactive = TRUE, retired_at = <prev>`, silently leaving the wallet
        // hidden from /providers and blocked from `apply_seller_discovery` (which
        // filters on `retired_at IS NULL`). `listing_opt_in` is deliberately NOT
        // reset here; re-opting in is a separate explicit action via the optional
        // discovery payload on POST /onboard.
        const SQL: &str = r#"
                INSERT INTO resource_providers (
                    wallet_pubkey, settlement_mode, spl_mint,
                    split_vault_pda, vault_sol_storage_pda, updated_at, registration_verified_at
                )
                VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
                ON CONFLICT (wallet_pubkey, settlement_mode, spl_mint) DO UPDATE SET
                    split_vault_pda = EXCLUDED.split_vault_pda,
                    vault_sol_storage_pda = EXCLUDED.vault_sol_storage_pda,
                    updated_at = NOW(),
                    registration_verified_at = NOW(),
                    retired_at = NULL,
                    inactive = FALSE
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
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_escrow_detail(
        &self,
        correlation_id: &str,
        escrow_pda: &str,
        bank_pda: &str,
        oracle_authority: &str,
        sla_hash: Option<&str>,
        fund_signature: Option<&str>,
        payment_uid_hex: Option<&str>,
    ) -> Result<(), DbError> {
        const SELECT_ID: &str = r#"SELECT id FROM payment_attempts WHERE correlation_id = $1"#;
        const SQL: &str = r#"
                INSERT INTO escrow_details (
                    payment_attempt_id, escrow_pda, bank_pda, oracle_authority,
                    sla_hash, fund_signature, payment_uid_hex, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
                ON CONFLICT (payment_attempt_id) DO UPDATE SET
                    escrow_pda = EXCLUDED.escrow_pda,
                    bank_pda = EXCLUDED.bank_pda,
                    oracle_authority = EXCLUDED.oracle_authority,
                    sla_hash = COALESCE(EXCLUDED.sla_hash, escrow_details.sla_hash),
                    fund_signature = COALESCE(EXCLUDED.fund_signature, escrow_details.fund_signature),
                    payment_uid_hex = COALESCE(EXCLUDED.payment_uid_hex, escrow_details.payment_uid_hex),
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
                    &payment_uid_hex,
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

    /// Count how many shadow vaults (facilitator-paid) were created in the last 24 hours.
    pub async fn count_daily_vault_creations(&self) -> Result<u64, DbError> {
        // We count resource_providers where the facilitator provisioned (verified_at is null)
        // and a vault PDA exists, within the last 24 hours.
        const SQL: &str = r#"
                SELECT COUNT(*) as count
                FROM resource_providers
                WHERE registration_verified_at IS NULL
                  AND split_vault_pda IS NOT NULL
                  AND created_at >= NOW() - INTERVAL '24 hours'
                "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(Self::QUERY_TIMEOUT, tx.query_one(SQL, &[])).await {
            Ok(Ok(row)) => {
                let count: i64 = row.get("count");
                Ok(count as u64)
            }
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "count_daily_vault_creations failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => Err(DbError::Timeout),
        };

        if result.is_ok() {
            tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        } else {
            tx.rollback().await.ok();
        }
        result
    }
    pub async fn get_resource_provider_sweep_threshold(
        &self,
        wallet_pubkey_str: &str,
        spl_mint_str: Option<&str>,
    ) -> Result<Option<u64>, DbError> {
        const SQL: &str = r#"
            SELECT sweep_threshold
            FROM resource_providers
            WHERE wallet_pubkey = $1
              AND spl_mint IS NOT DISTINCT FROM $2
            ORDER BY id DESC
            LIMIT 1
        "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(
            Self::QUERY_TIMEOUT,
            tx.query_opt(SQL, &[&wallet_pubkey_str, &spl_mint_str]),
        )
        .await
        {
            Ok(Ok(Some(row))) => {
                let threshold: Option<i64> = row.get("sweep_threshold");
                Ok(threshold.map(|v| v as u64))
            }
            Ok(Ok(None)) => Ok(None),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "get_resource_provider_sweep_threshold failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => Err(DbError::Timeout),
        };

        if result.is_ok() {
            tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        } else {
            tx.rollback().await.ok();
        }
        result
    }

    /// List active provider rails eligible for cron sweep checks.
    pub async fn list_sweep_candidates(
        &self,
        cooldown_sec: u64,
        recent_settle_window_sec: u64,
        limit: u64,
    ) -> Result<Vec<SweepCandidate>, DbError> {
        const SQL: &str = r#"
            SELECT
                rp.wallet_pubkey,
                rp.settlement_mode,
                rp.spl_mint,
                rp.sweep_threshold
            FROM resource_providers rp
            WHERE rp.inactive = false
              AND rp.split_vault_pda IS NOT NULL
              AND (
                    rp.last_sweep_attempt_at IS NULL
                    OR rp.last_sweep_attempt_at < NOW() - ($1::BIGINT * INTERVAL '1 second')
              )
              AND EXISTS (
                    SELECT 1
                    FROM payment_attempts pa
                    WHERE pa.resource_provider_id = rp.id
                      AND pa.settle_ok = true
                      AND pa.settle_at IS NOT NULL
                      AND pa.settle_at > NOW() - ($2::BIGINT * INTERVAL '1 second')
              )
            ORDER BY COALESCE(rp.last_sweep_attempt_at, TO_TIMESTAMP(0)) ASC
            LIMIT $3
        "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(
            Self::QUERY_TIMEOUT,
            tx.query(
                SQL,
                &[
                    &(cooldown_sec as i64),
                    &(recent_settle_window_sec as i64),
                    &(limit as i64),
                ],
            ),
        )
        .await
        {
            Ok(Ok(rows)) => {
                let mut out = Vec::with_capacity(rows.len());
                for row in rows {
                    let threshold: Option<i64> = row.get("sweep_threshold");
                    out.push(SweepCandidate {
                        wallet_pubkey: row.get("wallet_pubkey"),
                        settlement_mode: row.get("settlement_mode"),
                        spl_mint: row.get("spl_mint"),
                        sweep_threshold: threshold.map(|v| v as u64),
                    });
                }
                Ok(out)
            }
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "list_sweep_candidates failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => Err(DbError::Timeout),
        };

        if result.is_ok() {
            tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        } else {
            tx.rollback().await.ok();
        }
        result
    }

    /// Record that a sweep was attempted for a provider rail; optionally store successful signature.
    pub async fn record_sweep_attempt(
        &self,
        wallet_pubkey: &str,
        settlement_mode: &str,
        spl_mint: Option<&str>,
        sweep_signature: Option<&str>,
    ) -> Result<(), DbError> {
        const SQL: &str = r#"
            UPDATE resource_providers
            SET
                last_sweep_attempt_at = NOW(),
                last_sweep_signature = COALESCE($4, last_sweep_signature),
                updated_at = NOW()
            WHERE wallet_pubkey = $1
              AND settlement_mode = $2
              AND spl_mint IS NOT DISTINCT FROM $3
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
                    &wallet_pubkey,
                    &settlement_mode,
                    &spl_mint,
                    &sweep_signature,
                ],
            ),
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "record_sweep_attempt failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => Err(DbError::Timeout),
        };

        if result.is_ok() {
            tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        } else {
            tx.rollback().await.ok();
        }
        result
    }

    /// List funded sla-escrow payments that may be eligible for the
    /// permissionless settlement cron. The handler then reads each
    /// candidate's on-chain `Payment` PDA to make the actual decision
    /// per `oracles/spec/sla-escrow-onchain-abi/v1/NORMATIVE.md` §5.3 / §5.4.
    ///
    /// Selection criteria:
    /// - `escrow_details.fund_signature IS NOT NULL` (FundPayment landed
    ///   per pr402's record).
    /// - `escrow_details.completed_at IS NULL` AND `refunded_at IS NULL`
    ///   (pr402 has not yet recorded a successful settlement).
    /// - `escrow_details.payment_uid_hex IS NOT NULL` (pre-migration
    ///   rows are skipped; the cron needs the uid to derive the Payment
    ///   PDA).
    /// - `updated_at < NOW() - cooldown_sec` (per-row cooldown).
    /// - `created_at > NOW() - lookback_sec` (bound the search; older
    ///   payments may have been settled outside pr402's view).
    pub async fn list_sla_escrow_settle_candidates(
        &self,
        cooldown_sec: u64,
        lookback_sec: u64,
        limit: u64,
    ) -> Result<Vec<SlaEscrowSettleCandidate>, DbError> {
        const SQL: &str = r#"
            SELECT
                pa.correlation_id,
                ed.payment_uid_hex,
                ed.escrow_pda,
                ed.bank_pda,
                ed.oracle_authority,
                pa.payer_wallet,
                pa.asset
            FROM escrow_details ed
            INNER JOIN payment_attempts pa ON pa.id = ed.payment_attempt_id
            WHERE ed.fund_signature IS NOT NULL
              AND ed.completed_at IS NULL
              AND ed.refunded_at IS NULL
              AND ed.payment_uid_hex IS NOT NULL
              AND ed.updated_at < NOW() - ($1::BIGINT * INTERVAL '1 second')
              AND ed.created_at > NOW() - ($2::BIGINT * INTERVAL '1 second')
            ORDER BY ed.updated_at ASC
            LIMIT $3
        "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(
            Self::QUERY_TIMEOUT,
            tx.query(
                SQL,
                &[
                    &(cooldown_sec as i64),
                    &(lookback_sec as i64),
                    &(limit as i64),
                ],
            ),
        )
        .await
        {
            Ok(Ok(rows)) => {
                let mut out = Vec::with_capacity(rows.len());
                for row in rows {
                    let mint: Option<String> = row.get("asset");
                    out.push(SlaEscrowSettleCandidate {
                        correlation_id: row.get("correlation_id"),
                        payment_uid_hex: row.get("payment_uid_hex"),
                        escrow_pda: row.get("escrow_pda"),
                        bank_pda: row.get("bank_pda"),
                        oracle_authority: row.get("oracle_authority"),
                        // `pa.payer_wallet` is the buyer (sender of FundPayment).
                        // The seller comes from the on-chain Payment PDA.
                        buyer_wallet: row.get("payer_wallet"),
                        seller_wallet: None,
                        mint,
                    });
                }
                Ok(out)
            }
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "list_sla_escrow_settle_candidates failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => Err(DbError::Timeout),
        };

        if result.is_ok() {
            tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        } else {
            tx.rollback().await.ok();
        }
        result
    }

    /// Bump `escrow_details.updated_at` after a settlement-cron attempt
    /// (success or failure). Drives the per-row cooldown enforced by
    /// `list_sla_escrow_settle_candidates`.
    pub async fn touch_sla_escrow_settle_attempt(
        &self,
        correlation_id: &str,
    ) -> Result<(), DbError> {
        const SQL: &str = r#"
            UPDATE escrow_details
            SET updated_at = NOW()
            WHERE payment_attempt_id = (
                SELECT id FROM payment_attempts WHERE correlation_id = $1
            )
        "#;

        let mut client = self.conn().await?;
        let tx = client.transaction().await.map_err(|e| {
            error!(target: "server_log", error = %e, "Transaction start failed");
            DbError::TransactionFailed
        })?;

        Self::deallocate_all_signer_style(&tx).await;

        let result = match timeout(Self::QUERY_TIMEOUT, tx.execute(SQL, &[&correlation_id])).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => {
                error!(target: "server_log", error = %format_err_chain(&e), "touch_sla_escrow_settle_attempt failed");
                Err(DbError::Query(format_err_chain(&e)))
            }
            Err(_) => Err(DbError::Timeout),
        };

        if result.is_ok() {
            tx.commit().await.map_err(|_| DbError::TransactionFailed)?;
        } else {
            tx.rollback().await.ok();
        }
        result
    }
}
