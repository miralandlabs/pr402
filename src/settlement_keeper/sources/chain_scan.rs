//! Chain-first candidate discovery via `getProgramAccounts` on Payment PDAs.

use async_trait::async_trait;
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;

use crate::chain::solana_sla_escrow::derive_bank_pda;
use crate::db::SlaEscrowSettleCandidate;
use crate::settlement_keeper::payment::decode_payment_view;
use crate::settlement_keeper::sources::SlaEscrowSettleCandidateSource;

const PAYMENT_DISCRIMINATOR: u8 = 103;
const PAYMENT_ACCOUNT_LEN: u64 = 384; // 8-byte header + 376-byte body
const PAYMENT_STATE_ACCOUNT_OFFSET: usize = 382; // header 8 + body offset 374

fn payment_scan_config(filters: Vec<RpcFilterType>) -> RpcProgramAccountsConfig {
    RpcProgramAccountsConfig {
        filters: Some(filters),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::confirmed()),
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn load_program_account_data(
    rpc: &RpcClient,
    program_id: &Pubkey,
    config: RpcProgramAccountsConfig,
) -> Result<Vec<Vec<u8>>, String> {
    let rows = rpc
        .get_program_ui_accounts_with_config(program_id, config)
        .await
        .map_err(|e| format!("getProgramAccounts failed: {}", e))?;

    Ok(rows
        .into_iter()
        .filter_map(|(_pk, ui)| ui.data.decode())
        .collect())
}

/// Scan funded Payment PDAs directly from chain (no pr402 DB required).
pub struct ChainScanSettleSource {
    pub rpc: RpcClient,
    pub program_id: Pubkey,
    pub bank_pda: Pubkey,
}

impl ChainScanSettleSource {
    pub fn new(rpc_url: &str, program_id: Pubkey) -> Self {
        let (bank_pda, _) = derive_bank_pda(&program_id);
        Self {
            rpc: RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed()),
            program_id,
            bank_pda,
        }
    }

    async fn scan_funded_payments(
        &self,
        limit: u64,
    ) -> Result<Vec<SlaEscrowSettleCandidate>, String> {
        let config = payment_scan_config(vec![
            RpcFilterType::DataSize(PAYMENT_ACCOUNT_LEN),
            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(0, vec![PAYMENT_DISCRIMINATOR])),
            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
                PAYMENT_STATE_ACCOUNT_OFFSET,
                vec![0], // Funded
            )),
        ]);

        let accounts = load_program_account_data(&self.rpc, &self.program_id, config).await?;

        let mut out = Vec::new();
        for data in accounts {
            if out.len() >= limit as usize {
                break;
            }
            let view = match decode_payment_view(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if view.state != 0 {
                continue;
            }
            let payment_uid_hex = view.payment_uid_hex();
            out.push(SlaEscrowSettleCandidate {
                correlation_id: payment_uid_hex.clone(),
                payment_uid_hex,
                escrow_pda: String::new(),
                bank_pda: self.bank_pda.to_string(),
                mint: if view.mint == Pubkey::default() {
                    None
                } else {
                    Some(view.mint.to_string())
                },
                buyer_wallet: Some(view.buyer.to_string()),
                seller_wallet: Some(view.seller.to_string()),
                oracle_authority: view.oracle_authority.to_string(),
            });
        }
        Ok(out)
    }
}

#[async_trait]
impl SlaEscrowSettleCandidateSource for ChainScanSettleSource {
    async fn list_sla_escrow_settle_candidates(
        &self,
        _cooldown_sec: u64,
        _lookback_sec: u64,
        limit: u64,
    ) -> Result<Vec<SlaEscrowSettleCandidate>, String> {
        self.scan_funded_payments(limit).await
    }
}

/// Terminal Payment PDAs eligible for `ClosePayment` (state != Funded, closure delay elapsed).
pub struct ChainScanCloseSource {
    pub rpc: RpcClient,
    pub program_id: Pubkey,
    pub bank_pda: Pubkey,
}

impl ChainScanCloseSource {
    pub fn new(rpc_url: &str, program_id: Pubkey) -> Self {
        let (bank_pda, _) = derive_bank_pda(&program_id);
        Self {
            rpc: RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed()),
            program_id,
            bank_pda,
        }
    }

    pub async fn scan_close_candidates(
        &self,
        now_unix: i64,
        limit: u64,
    ) -> Result<Vec<SlaEscrowSettleCandidate>, String> {
        let config = payment_scan_config(vec![
            RpcFilterType::DataSize(PAYMENT_ACCOUNT_LEN),
            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(0, vec![PAYMENT_DISCRIMINATOR])),
        ]);

        let accounts = load_program_account_data(&self.rpc, &self.program_id, config).await?;

        let mut out = Vec::new();
        for data in accounts {
            if out.len() >= limit as usize {
                break;
            }
            let view = match decode_payment_view(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if view.state == 0 {
                continue;
            }
            if now_unix <= view.closed_at {
                continue;
            }
            let payment_uid_hex = view.payment_uid_hex();
            out.push(SlaEscrowSettleCandidate {
                correlation_id: payment_uid_hex.clone(),
                payment_uid_hex,
                escrow_pda: String::new(),
                bank_pda: self.bank_pda.to_string(),
                mint: if view.mint == Pubkey::default() {
                    None
                } else {
                    Some(view.mint.to_string())
                },
                buyer_wallet: Some(view.buyer.to_string()),
                seller_wallet: Some(view.seller.to_string()),
                oracle_authority: view.oracle_authority.to_string(),
            });
        }
        Ok(out)
    }
}
