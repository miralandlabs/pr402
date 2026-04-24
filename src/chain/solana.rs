//! Solana chain provider for x402 facilitator.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::{RpcSendTransactionConfig, RpcSimulateTransactionConfig};
use solana_client::rpc_response::{Response, RpcSimulateTransactionResult};
use solana_commitment_config::CommitmentConfig;
use solana_keypair::{Keypair, Signer};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction::TransactionError;

// System Program ID
pub const SYSTEM_PROGRAM_ID: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");
pub const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const TOKEN_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
/// SPL Token-2022 program (`spl_token_2022::ID`); kept here to avoid the `spl-token-2022` crate (broken 10.0.0 publish vs `spl-token-group-interface`).
pub const TOKEN_2022_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// A Solana chain reference consisting of 32 ASCII characters.
#[derive(Clone, Debug, PartialEq)]
pub struct SolanaChainReference(pub String);

impl SolanaChainReference {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SolanaChainReference {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SolanaChainProviderError {
    #[error("RPC error: {0}")]
    Transport(String),
    #[error("Transaction error: {0}")]
    Transaction(#[from] TransactionError),
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(#[from] solana_client::rpc_response::UiTransactionError),
    #[error("Signer not found in transaction")]
    SignerNotFoundInTransaction,
    #[error("Account not found")]
    AccountNotFound,
}

impl From<solana_client::client_error::ClientError> for SolanaChainProviderError {
    fn from(value: solana_client::client_error::ClientError) -> Self {
        Self::Transport(value.to_string())
    }
}

pub struct SolanaChainProvider {
    pub(crate) rpc_client: RpcClient,
    pub(crate) keypair: Arc<Keypair>,
    pub(crate) chain_id: crate::chain::ChainId,
    pub(crate) rpc_url: String,
    pub(crate) universalsettle: Option<crate::config::UniversalSettleConfig>,
    pub(crate) escrow: Option<crate::config::SLAEscrowConfig>,
    pub(crate) max_compute_unit_limit: u32,
    pub(crate) max_compute_unit_price: u64,
}

impl SolanaChainProvider {
    pub fn new(
        rpc_url: &str,
        keypair: Keypair,
        chain_id: crate::chain::ChainId,
        universalsettle: Option<crate::config::UniversalSettleConfig>,
        escrow: Option<crate::config::SLAEscrowConfig>,
        max_compute_unit_limit: u32,
        max_compute_unit_price: u64,
    ) -> Self {
        Self {
            rpc_client: RpcClient::new(rpc_url.to_string()),
            keypair: Arc::new(keypair),
            chain_id,
            rpc_url: rpc_url.to_string(),
            universalsettle,
            escrow,
            max_compute_unit_limit,
            max_compute_unit_price,
        }
    }

    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    pub async fn get_health(&self) -> Result<u64, SolanaChainProviderError> {
        // We use get_slot as a proxy for 'active/responsive' health.
        self.rpc_client.get_slot().await.map_err(|e| e.into())
    }

    pub async fn account_exists(&self, pubkey: &Pubkey) -> Result<bool, SolanaChainProviderError> {
        match self.rpc_client.get_account(pubkey).await {
            Ok(_) => Ok(true),
            Err(e) => {
                if e.to_string().contains("AccountNotFound") {
                    Ok(false)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    pub fn chain_id(&self) -> crate::chain::ChainId {
        self.chain_id.clone()
    }

    pub fn max_compute_unit_limit(&self) -> u32 {
        self.max_compute_unit_limit
    }

    pub fn max_compute_unit_price(&self) -> u64 {
        self.max_compute_unit_price
    }

    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    pub fn rpc_client(&self) -> &RpcClient {
        &self.rpc_client
    }

    pub fn universalsettle(&self) -> Option<&crate::config::UniversalSettleConfig> {
        self.universalsettle.as_ref()
    }

    pub fn sla_escrow(&self) -> Option<&crate::config::SLAEscrowConfig> {
        self.escrow.as_ref()
    }

    pub fn get_vault_pda(&self, seller: &Pubkey) -> (Pubkey, u8) {
        let program_id = self
            .universalsettle()
            .map(|u| u.program_id)
            .unwrap_or(SYSTEM_PROGRAM_ID);
        Pubkey::find_program_address(&[b"vault", seller.as_ref()], &program_id)
    }

    pub fn get_sol_storage_pda(&self, vault_pda: Pubkey) -> (Pubkey, u8) {
        let program_id = self
            .universalsettle()
            .map(|u| u.program_id)
            .unwrap_or(SYSTEM_PROGRAM_ID);
        Pubkey::find_program_address(&[b"sol_storage", vault_pda.as_ref()], &program_id)
    }

    pub fn get_sla_escrow_sol_storage_pda(
        &self,
        mint: Pubkey,
        bank: Pubkey,
        escrow: Pubkey,
    ) -> (Pubkey, u8) {
        let program_id = self
            .sla_escrow()
            .map(|e| e.program_id)
            .unwrap_or(SYSTEM_PROGRAM_ID);
        Pubkey::find_program_address(
            &[
                b"sol_storage",
                mint.as_ref(),
                bank.as_ref(),
                escrow.as_ref(),
            ],
            &program_id,
        )
    }

    pub fn get_escrow_pda(&self, mint: Pubkey, bank: Pubkey) -> (Pubkey, u8) {
        let program_id = self
            .sla_escrow()
            .map(|e| e.program_id)
            .unwrap_or(SYSTEM_PROGRAM_ID);
        Pubkey::find_program_address(&[b"escrow", mint.as_ref(), bank.as_ref()], &program_id)
    }

    pub fn get_payment_pda(&self, payment_uid: &str, bank: Pubkey) -> (Pubkey, u8) {
        let program_id = self
            .sla_escrow()
            .map(|e| e.program_id)
            .unwrap_or(SYSTEM_PROGRAM_ID);
        let uid_bytes = crate::chain::solana_sla_escrow::sanitize_uid(payment_uid);
        Pubkey::find_program_address(&[b"payment", &uid_bytes, bank.as_ref()], &program_id)
    }

    pub fn get_bank_pda(&self, program_id: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"bank"], program_id)
    }

    pub fn get_config_pda(&self, program_id: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"config"], program_id)
    }

    pub async fn ensure_sla_escrow_setup(
        &self,
        mint: Pubkey,
    ) -> Result<Pubkey, SolanaChainProviderError> {
        let escrow_config = self
            .sla_escrow()
            .ok_or(SolanaChainProviderError::Transport(
                "SLA Escrow not configured".to_string(),
            ))?;
        let bank_pda = escrow_config
            .bank_address
            .ok_or(SolanaChainProviderError::Transport(
                "SLA Escrow bank not loaded".to_string(),
            ))?;

        let (escrow_pda, _bump) = self.get_escrow_pda(mint, bank_pda);

        // Ensure Escrow account exists for this mint
        let account = self.rpc_client.get_account(&escrow_pda).await;
        if account.is_err() {
            tracing::info!(escrow = %escrow_pda, mint = %mint, "Creating Escrow account for mint");
            // NOTE: Usually Escrow accounts are created by an Admin through a separate instruction.
            // If the facilitator is also the admin, we could create it here.
            // For now, we'll just check if it exists and return error if not,
            // OR return the address anyway if we want caller to create it.
            return Err(SolanaChainProviderError::Transport(format!(
                "SLA Escrow account for mint {} not found on-chain",
                mint
            )));
        }

        Ok(escrow_pda)
    }

    pub async fn ensure_vault_setup(
        &self,
        seller: &Pubkey,
        _fee_destination: &Pubkey,
        _fee_bps: u16,
        mint: Option<Pubkey>,
    ) -> Result<Pubkey, SolanaChainProviderError> {
        let (vault_pda, _bump) = self.get_vault_pda(seller);

        // 1. Ensure SplitVault State PDA exists
        let account = self.rpc_client.get_account(&vault_pda).await;
        if account.is_err() {
            tracing::info!(vault = %vault_pda, "Creating SplitVault PDA");
            if let Some(us_config) = self.universalsettle() {
                let program_id = us_config.program_id;
                let mut data = vec![1]; // CreateVault discriminator is 1
                data.extend_from_slice(seller.as_ref());

                let (vault_sol_storage, _) = self.get_sol_storage_pda(vault_pda);

                let ix = solana_transaction::Instruction {
                    program_id,
                    accounts: vec![
                        solana_transaction::AccountMeta::new(self.pubkey(), true),
                        solana_transaction::AccountMeta::new(vault_pda, false),
                        solana_transaction::AccountMeta::new(vault_sol_storage, false),
                        solana_transaction::AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                    ],
                    data,
                };

                let recent_blockhash = self.rpc_client().get_latest_blockhash().await?;
                let tx = VersionedTransaction::from(
                    solana_transaction::Transaction::new_signed_with_payer(
                        &[ix],
                        Some(&self.pubkey()),
                        &[self.keypair()],
                        recent_blockhash,
                    ),
                );
                self.send_and_confirm(&tx, CommitmentConfig::confirmed())
                    .await?;
            }
        }

        // 2. Ensure ATA for Vault exists if payment is in SPL tokens (legacy Token or plain Token-2022 mint).
        if let Some(mint_key) = mint {
            if mint_key != Pubkey::default() {
                let mint_acc = self
                    .rpc_client
                    .get_account(&mint_key)
                    .await
                    .map_err(|e| SolanaChainProviderError::Transport(e.to_string()))?;
                let token_program = if mint_acc.owner == spl_token::ID {
                    spl_token::ID
                } else if mint_acc.owner == TOKEN_2022_PROGRAM_ID {
                    TOKEN_2022_PROGRAM_ID
                } else {
                    return Err(SolanaChainProviderError::Transport(format!(
                        "mint {} owner {} is not spl_token or token-2022",
                        mint_key, mint_acc.owner
                    )));
                };

                let (ata, _) = Pubkey::find_program_address(
                    &[
                        vault_pda.as_ref(),
                        token_program.as_ref(),
                        mint_key.as_ref(),
                    ],
                    &ASSOCIATED_TOKEN_PROGRAM_ID,
                );

                let ata_account = self.rpc_client.get_account(&ata).await;
                if ata_account.is_err() {
                    tracing::info!(vault = %vault_pda, ata = %ata, "Creating Vault ATA for SPL tokens");

                    let ata_ix = solana_transaction::Instruction {
                        program_id: ASSOCIATED_TOKEN_PROGRAM_ID,
                        accounts: vec![
                            solana_transaction::AccountMeta::new(self.pubkey(), true),
                            solana_transaction::AccountMeta::new(ata, false),
                            solana_transaction::AccountMeta::new_readonly(vault_pda, false),
                            solana_transaction::AccountMeta::new_readonly(mint_key, false),
                            solana_transaction::AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                            solana_transaction::AccountMeta::new_readonly(token_program, false),
                        ],
                        data: vec![1], // CreateIdempotent
                    };

                    let recent_blockhash = self.rpc_client().get_latest_blockhash().await?;
                    let tx = VersionedTransaction::from(
                        solana_transaction::Transaction::new_signed_with_payer(
                            &[ata_ix],
                            Some(&self.pubkey()),
                            &[self.keypair()],
                            recent_blockhash,
                        ),
                    );
                    self.send_and_confirm(&tx, CommitmentConfig::confirmed())
                        .await?;
                }
            }
        }

        Ok(vault_pda)
    }

    pub fn fee_payer(&self) -> Pubkey {
        self.pubkey()
    }

    pub fn signer_addresses(&self) -> Vec<Pubkey> {
        vec![self.pubkey()]
    }

    pub fn sign(
        &self,
        mut tx: VersionedTransaction,
    ) -> Result<VersionedTransaction, SolanaChainProviderError> {
        let pk = self.pubkey();
        if let Some(idx) = tx
            .message
            .static_account_keys()
            .iter()
            .position(|k| *k == pk)
        {
            tx.signatures[idx] = self.keypair.sign_message(tx.message.serialize().as_slice());
            Ok(tx)
        } else {
            Err(SolanaChainProviderError::SignerNotFoundInTransaction)
        }
    }

    /// Submit without local simulation (facilitator chooses when fee/speed favors skipping preflight).
    pub async fn send(
        &self,
        tx: &VersionedTransaction,
    ) -> Result<Signature, SolanaChainProviderError> {
        let cfg = RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: Some(CommitmentConfig::confirmed().commitment),
            encoding: None,
            max_retries: None,
            min_context_slot: None,
        };
        self.rpc_client
            .send_transaction_with_config(tx, cfg)
            .await
            .map_err(|e| e.into())
    }

    /// UniversalSettle sweep: run RPC preflight so facilitator fee is less likely spent on a doomed tx.
    pub async fn send_sweep_transaction(
        &self,
        tx: &VersionedTransaction,
    ) -> Result<Signature, SolanaChainProviderError> {
        let cfg = RpcSendTransactionConfig {
            skip_preflight: false,
            preflight_commitment: Some(CommitmentConfig::confirmed().commitment),
            encoding: None,
            max_retries: None,
            min_context_slot: None,
        };
        self.rpc_client
            .send_transaction_with_config(tx, cfg)
            .await
            .map_err(|e| e.into())
    }

    pub async fn send_and_confirm(
        &self,
        tx: &VersionedTransaction,
        commitment_config: CommitmentConfig,
    ) -> Result<Signature, SolanaChainProviderError> {
        self.rpc_client
            .send_and_confirm_transaction_with_spinner_and_commitment(tx, commitment_config)
            .await
            .map_err(|e| e.into())
    }

    pub async fn simulate_transaction_with_config(
        &self,
        tx: &VersionedTransaction,
        cfg: RpcSimulateTransactionConfig,
    ) -> Result<Response<RpcSimulateTransactionResult>, SolanaChainProviderError> {
        self.rpc_client
            .simulate_transaction_with_config(tx, cfg)
            .await
            .map_err(|e| e.into())
    }

    pub fn extract_transfer_from_pst(
        &self,
        tx: &VersionedTransaction,
        payer: &Pubkey,
    ) -> Result<TransferDetails, SolanaChainProviderError> {
        let instructions = tx.message.instructions();
        let last_ix = instructions
            .last()
            .ok_or(SolanaChainProviderError::AccountNotFound)?;
        let data = &last_ix.data;
        let account_keys = tx.message.static_account_keys();
        let program_id = *last_ix.program_id(account_keys);

        if program_id == TOKEN_PROGRAM_ID || program_id == TOKEN_2022_PROGRAM_ID {
            if data.len() < 9 {
                return Err(SolanaChainProviderError::AccountNotFound);
            }
            let amount = u64::from_le_bytes(data[1..9].try_into().unwrap());
            let _source = account_keys[*last_ix
                .accounts
                .first()
                .ok_or(SolanaChainProviderError::AccountNotFound)?
                as usize];
            let mint = account_keys[*last_ix
                .accounts
                .get(1)
                .ok_or(SolanaChainProviderError::AccountNotFound)?
                as usize];
            let destination = account_keys[*last_ix
                .accounts
                .get(2)
                .ok_or(SolanaChainProviderError::AccountNotFound)?
                as usize];
            let authority = account_keys[*last_ix
                .accounts
                .get(3)
                .ok_or(SolanaChainProviderError::AccountNotFound)?
                as usize];

            if &authority != payer {
                return Err(SolanaChainProviderError::AccountNotFound);
            }
            Ok(TransferDetails {
                payer: authority,
                payee: destination,
                amount,
                mint: Some(mint),
                token_program: Some(program_id),
            })
        } else if program_id == SYSTEM_PROGRAM_ID {
            if data.len() < 12 {
                return Err(SolanaChainProviderError::AccountNotFound);
            }
            let amount = u64::from_le_bytes(data[4..12].try_into().unwrap());
            let source = account_keys[*last_ix
                .accounts
                .first()
                .ok_or(SolanaChainProviderError::AccountNotFound)?
                as usize];
            let destination = account_keys[*last_ix
                .accounts
                .get(1)
                .ok_or(SolanaChainProviderError::AccountNotFound)?
                as usize];

            if &source != payer {
                return Err(SolanaChainProviderError::AccountNotFound);
            }
            Ok(TransferDetails {
                payer: source,
                payee: destination,
                amount,
                mint: None,
                token_program: None,
            })
        } else {
            Err(SolanaChainProviderError::AccountNotFound)
        }
    }
}

pub struct TransferDetails {
    pub payer: Pubkey,
    pub payee: Pubkey,
    pub amount: u64,
    pub mint: Option<Pubkey>,
    /// SPL Token or Token-2022 program ID for SPL transfers; `None` for native SOL.
    pub token_program: Option<Pubkey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Address(Pubkey);

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let pubkey = Pubkey::from_str(&s).map_err(serde::de::Error::custom)?;
        Ok(Address(pubkey))
    }
}

impl Address {
    pub fn new(pubkey: Pubkey) -> Self {
        Self(pubkey)
    }

    pub fn pubkey(&self) -> &Pubkey {
        &self.0
    }
}

impl From<Pubkey> for Address {
    fn from(p: Pubkey) -> Self {
        Self(p)
    }
}

impl From<Address> for Pubkey {
    fn from(a: Address) -> Self {
        a.0
    }
}
