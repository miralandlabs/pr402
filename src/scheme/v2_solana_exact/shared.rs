//! Shared Solana verification and settlement logic for v2:solana:exact.

use solana_client::rpc_response::UiTransactionError;
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ID as ComputeBudgetInstructionId;
use solana_message::compiled_instruction::CompiledInstruction;
use solana_pubkey::{pubkey, Pubkey};
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction::TransactionError;

use crate::chain::solana::{Address, SolanaChainProvider, SolanaChainProviderError};
use crate::proto::PaymentVerificationError;
use crate::util::Base64Bytes;

pub const ATA_PROGRAM_PUBKEY: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

#[derive(Clone, Debug)]
pub struct TransferRequirement<'a> {
    pub asset: &'a Address,
    pub pay_to: &'a Address,
    pub amount: u64,
}

fn is_ata_of(ata: Pubkey, owner: Pubkey, mint: &Pubkey, token_program: &Pubkey) -> bool {
    if mint == &Pubkey::default() {
        return false;
    }
    let (derived, _) = Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ATA_PROGRAM_PUBKEY,
    );
    ata == derived
}

pub struct VerifyTransferResult {
    pub payer: Address,
    pub beneficiary: Address,
    pub transaction: VersionedTransaction,
}

pub struct InstructionInt {
    index: usize,
    instruction: CompiledInstruction,
    account_keys: Vec<Pubkey>,
}

impl InstructionInt {
    pub fn has_data(&self) -> bool {
        !self.instruction.data.is_empty()
    }

    pub fn has_accounts(&self) -> bool {
        !self.instruction.accounts.is_empty()
    }

    pub fn data_slice(&self) -> &[u8] {
        self.instruction.data.as_slice()
    }

    pub fn assert_not_empty(&self) -> Result<(), SolanaExactError> {
        if !self.has_data() || !self.has_accounts() {
            return Err(SolanaExactError::EmptyInstructionAtIndex(self.index));
        }
        Ok(())
    }

    pub fn program_id(&self) -> Pubkey {
        *self.instruction.program_id(self.account_keys.as_slice())
    }

    pub fn account(&self, index: u8) -> Result<Pubkey, SolanaExactError> {
        let account_index = self
            .instruction
            .accounts
            .get(index as usize)
            .cloned()
            .ok_or(SolanaExactError::NoAccountAtIndex(index))?;
        let pubkey = self
            .account_keys
            .get(account_index as usize)
            .cloned()
            .ok_or(SolanaExactError::NoAccountAtIndex(index))?;
        Ok(pubkey)
    }
}

pub struct TransactionInt {
    pub inner: VersionedTransaction,
}

impl TransactionInt {
    pub fn new(transaction: VersionedTransaction) -> Self {
        Self { inner: transaction }
    }

    pub fn instruction(&self, index: usize) -> Result<InstructionInt, SolanaExactError> {
        let instruction = self
            .inner
            .message
            .instructions()
            .get(index)
            .cloned()
            .ok_or(SolanaExactError::NoInstructionAtIndex(index))?;
        let account_keys = self.inner.message.static_account_keys().to_vec();

        Ok(InstructionInt {
            index,
            instruction,
            account_keys,
        })
    }

    pub fn is_fully_signed(&self) -> bool {
        let num_required = self.inner.message.header().num_required_signatures;
        if self.inner.signatures.len() < num_required as usize {
            return false;
        }
        let default = Signature::default();
        for signature in self.inner.signatures.iter() {
            if default.eq(signature) {
                return false;
            }
        }
        true
    }

    pub fn sign(self, provider: &SolanaChainProvider) -> Result<Self, SolanaChainProviderError> {
        let tx = provider.sign(self.inner)?;
        Ok(Self { inner: tx })
    }

    pub async fn send_and_confirm(
        &self,
        provider: &SolanaChainProvider,
        commitment_config: CommitmentConfig,
    ) -> Result<Signature, SolanaChainProviderError> {
        provider
            .send_and_confirm(&self.inner, commitment_config)
            .await
    }
}

#[derive(Debug)]
pub struct TransferCheckedInstruction {
    pub amount: u64,
    pub source: Pubkey,
    pub destination: Pubkey,
    pub authority: Pubkey,
    pub token_program: Pubkey,
}

pub fn verify_compute_limit_instruction(
    transaction: &VersionedTransaction,
    instruction_index: usize,
) -> Result<u32, SolanaExactError> {
    let instructions = transaction.message.instructions();
    let instruction = instructions
        .get(instruction_index)
        .ok_or(SolanaExactError::NoInstructionAtIndex(instruction_index))?;
    let account = instruction.program_id(transaction.message.static_account_keys());
    let data = instruction.data.as_slice();

    // Verify program ID, discriminator, and data length (1 byte discriminator + 4 bytes u32)
    if ComputeBudgetInstructionId.ne(account)
        || data.first().cloned().unwrap_or(0) != 2
        || data.len() != 5
    {
        return Err(SolanaExactError::InvalidComputeLimitInstruction);
    }

    // Parse compute unit limit (u32 in little-endian)
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&data[1..5]);
    let compute_units = u32::from_le_bytes(buf);

    Ok(compute_units)
}

pub fn verify_compute_price_instruction(
    max_compute_unit_price: u64,
    transaction: &VersionedTransaction,
    instruction_index: usize,
) -> Result<(), SolanaExactError> {
    let instructions = transaction.message.instructions();
    let instruction = instructions
        .get(instruction_index)
        .ok_or(SolanaExactError::NoInstructionAtIndex(instruction_index))?;
    let account = instruction.program_id(transaction.message.static_account_keys());
    let compute_budget = solana_compute_budget_interface::ID;
    let data = instruction.data.as_slice();
    if compute_budget.ne(account) || data.first().cloned().unwrap_or(0) != 3 || data.len() != 9 {
        return Err(SolanaExactError::InvalidComputePriceInstruction);
    }
    // It is ComputeBudgetInstruction definitely by now!
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[1..]);
    let microlamports = u64::from_le_bytes(buf);
    if microlamports > max_compute_unit_price {
        return Err(SolanaExactError::MaxComputeUnitPriceExceeded);
    }
    Ok(())
}

pub fn verify_create_ata_instruction(
    transaction: &VersionedTransaction,
    index: usize,
    transfer_requirement: &TransferRequirement<'_>,
) -> Result<(), PaymentVerificationError> {
    let tx = TransactionInt::new(transaction.clone());
    let instruction = tx.instruction(index)?;
    instruction.assert_not_empty()?;

    // Verify program ID is the Associated Token Account Program
    let program_id = instruction.program_id();
    if program_id != ATA_PROGRAM_PUBKEY {
        return Err(SolanaExactError::InvalidCreateATAInstruction.into());
    }

    // Verify instruction discriminator
    // The ATA program's Create instruction has discriminator 0 (Create) or 1 (CreateIdempotent)
    let data = instruction.data_slice();
    if data.is_empty() || (data[0] != 0 && data[0] != 1) {
        return Err(SolanaExactError::InvalidCreateATAInstruction.into());
    }

    // Verify account count (must have at least 6 accounts)
    if instruction.instruction.accounts.len() < 6 {
        return Err(SolanaExactError::InvalidCreateATAInstruction.into());
    }

    // Payer = 0, Owner = 2, Mint = 3
    let owner = instruction.account(2)?;
    let mint = instruction.account(3)?;

    // verify that the ATA is created for the expected payee
    if Address::new(owner) != *transfer_requirement.pay_to {
        return Err(PaymentVerificationError::RecipientMismatch);
    }
    if Address::new(mint) != *transfer_requirement.asset {
        return Err(PaymentVerificationError::AssetMismatch);
    }
    Ok(())
}

pub async fn verify_transaction(
    provider: &SolanaChainProvider,
    transaction_b64_string: String,
    transfer_requirement: &TransferRequirement<'_>,
) -> Result<VerifyTransferResult, PaymentVerificationError> {
    let bytes = Base64Bytes::from(transaction_b64_string.as_bytes())
        .decode()
        .map_err(|e| SolanaExactError::TransactionDecoding(e.to_string()))?;
    let transaction = bincode::deserialize::<VersionedTransaction>(bytes.as_slice())
        .map_err(|e| SolanaExactError::TransactionDecoding(e.to_string()))?;

    // perform transaction introspection to validate the transaction structure and details
    let instructions = transaction.message.instructions();
    let compute_units = verify_compute_limit_instruction(&transaction, 0)?;
    if compute_units > provider.max_compute_unit_limit() {
        return Err(SolanaExactError::MaxComputeUnitLimitExceeded.into());
    }
    verify_compute_price_instruction(provider.max_compute_unit_price(), &transaction, 1)?;

    let transfer_instruction = if instructions.len() == 3 {
        verify_transfer_instruction(provider, &transaction, 2, transfer_requirement, false).await?
    } else if instructions.len() == 4 {
        verify_create_ata_instruction(&transaction, 2, transfer_requirement)?;
        verify_transfer_instruction(provider, &transaction, 3, transfer_requirement, true).await?
    } else {
        return Err(SolanaExactError::InvalidTransactionInstructionsCount.into());
    };

    // Rule: UniversalSettle SplitVault enforcement
    if let Some(us_config) = provider.universalsettle() {
        let seller = *transfer_requirement.pay_to.pubkey();
        let _fee_dest = us_config
            .fee_destination
            .ok_or(SolanaExactError::UniversalSettleNotConfigured)?;
        let (vault_pda, _) = provider.get_vault_pda(&seller);

        let is_sol = *transfer_requirement.asset.pubkey() == Pubkey::default();
        let dest_match = if is_sol {
            let (vault_sol_storage, _) = provider.get_sol_storage_pda(vault_pda);
            transfer_instruction.destination == vault_sol_storage
        } else {
            is_ata_of(
                transfer_instruction.destination,
                vault_pda,
                transfer_requirement.asset.pubkey(),
                &transfer_instruction.token_program,
            )
        };

        if !dest_match {
            return Err(PaymentVerificationError::RecipientMismatch);
        }
    }

    // Rule 2: Fee payer safety check
    let fee_payer_pubkey = provider.pubkey();
    for instruction in transaction.message.instructions().iter() {
        for account_idx in instruction.accounts.iter() {
            let account = transaction
                .message
                .static_account_keys()
                .get(*account_idx as usize)
                .ok_or(SolanaExactError::NoAccountAtIndex(*account_idx))?;
            if *account == fee_payer_pubkey {
                return Err(SolanaExactError::FeePayerIncludedInInstructionAccounts.into());
            }
        }
    }

    let payer: Address = transfer_instruction.authority.into();
    let beneficiary = *transfer_requirement.pay_to;
    Ok(VerifyTransferResult {
        payer,
        beneficiary,
        transaction,
    })
}

pub async fn verify_transfer_instruction(
    provider: &SolanaChainProvider,
    transaction: &VersionedTransaction,
    instruction_index: usize,
    transfer_requirement: &TransferRequirement<'_>,
    _has_dest_ata: bool,
) -> Result<TransferCheckedInstruction, PaymentVerificationError> {
    let tx = TransactionInt::new(transaction.clone());
    let instruction = tx.instruction(instruction_index)?;
    instruction.assert_not_empty()?;
    let program_id = instruction.program_id();

    let transfer_checked_instruction = if spl_token::ID.eq(&program_id) {
        let token_instruction =
            spl_token::instruction::TokenInstruction::unpack(instruction.data_slice())
                .map_err(|_| SolanaExactError::InvalidTokenInstruction)?;
        let amount = match token_instruction {
            spl_token::instruction::TokenInstruction::TransferChecked {
                amount,
                decimals: _,
            } => amount,
            _ => return Err(SolanaExactError::InvalidTokenInstruction.into()),
        };
        TransferCheckedInstruction {
            amount,
            source: instruction.account(0)?,
            destination: instruction.account(2)?,
            authority: instruction.account(3)?,
            token_program: spl_token::ID,
        }
    } else if spl_token_2022::ID.eq(&program_id) {
        let token_instruction =
            spl_token_2022::instruction::TokenInstruction::unpack(instruction.data_slice())
                .map_err(|_| SolanaExactError::InvalidTokenInstruction)?;
        let amount = match token_instruction {
            spl_token_2022::instruction::TokenInstruction::TransferChecked {
                amount,
                decimals: _,
            } => amount,
            _ => return Err(SolanaExactError::InvalidTokenInstruction.into()),
        };
        TransferCheckedInstruction {
            amount,
            source: instruction.account(0)?,
            destination: instruction.account(2)?,
            authority: instruction.account(3)?,
            token_program: spl_token_2022::ID,
        }
    } else if crate::chain::solana::SYSTEM_PROGRAM_ID.eq(&program_id) {
        let instruction_data = instruction.data_slice();
        if instruction_data.len() < 12
            || u32::from_le_bytes(instruction_data[0..4].try_into().unwrap()) != 2
        {
            return Err(SolanaExactError::InvalidTokenInstruction.into());
        }
        let amount = u64::from_le_bytes(instruction_data[4..12].try_into().unwrap());
        TransferCheckedInstruction {
            amount,
            source: instruction.account(0)?,
            destination: instruction.account(1)?,
            authority: instruction.account(0)?,
            token_program: crate::chain::solana::SYSTEM_PROGRAM_ID,
        }
    } else {
        return Err(SolanaExactError::InvalidTokenInstruction.into());
    };

    let fee_payer_pubkey = provider.pubkey();
    if transfer_checked_instruction.authority == fee_payer_pubkey {
        return Err(SolanaExactError::FeePayerTransferringFunds.into());
    }

    // In SplitVault model, the "pay_to" in verification request might be the seller ADDRESS,
    // but the transaction destination might be the VAULT PDA (or its ATA).
    // The verify_transaction handles the vault match check.

    if transfer_checked_instruction.amount != transfer_requirement.amount {
        return Err(PaymentVerificationError::InvalidPaymentAmount);
    }

    Ok(transfer_checked_instruction)
}

/// Submit the payer's signed payment, then optionally submit a UniversalSettle **Sweep** (fee costs SOL).
///
/// **RPC preflight (why):** A sweep adds a second on-chain fee. Before building/sending it we call
/// [`crate::vault_balance::fetch_universalsettle_vault_snapshot`] on the **same** `RpcClient` used
/// for settlement, after `send_and_confirm` so balances include the just-settled transfer. If the
/// vault rail (SOL spendable lamports or SPL raw balance) is below configured floors from
/// [`crate::parameters`] we skip the sweep and only log. If snapshot RPC fails we still attempt the
/// sweep (fail-open) so funds are less likely to remain stuck in the vault.
///
/// **Parameters cache:** Serverless entrypoints should call [`crate::parameters::refresh_parameters_from_db`]
/// before settle so threshold env/DB values are loaded (see facilitator `handle_settle`).
///
/// **Sweep submit:** Uses [`SolanaChainProvider::send_sweep_transaction`] (RPC preflight on). Send errors
/// are logged and do not fail the overall settle (payment signature already returned to caller context).
pub async fn settle_transaction(
    provider: &SolanaChainProvider,
    verification: VerifyTransferResult,
) -> Result<Signature, SolanaChainProviderError> {
    let tx = TransactionInt::new(verification.transaction).sign(provider)?;
    if !tx.is_fully_signed() {
        return Err(SolanaChainProviderError::InvalidTransaction(
            UiTransactionError::from(TransactionError::SignatureFailure),
        ));
    }
    let tx_sig = tx
        .send_and_confirm(provider, CommitmentConfig::confirmed())
        .await?;

    if let Some(us_config) = provider.universalsettle() {
        let payer = *verification.payer.pubkey();
        match provider.extract_transfer_from_pst(&tx.inner, &payer) {
            Ok(details) => {
                let seller = *verification.beneficiary.pubkey();
                let fee_dest = us_config.fee_destination.unwrap_or_default();
                let (vault_pda, _) = provider.get_vault_pda(&seller);
                let (vault_sol_storage, _) = provider.get_sol_storage_pda(vault_pda);

                let spl_token_prog = details
                    .token_program
                    .as_ref()
                    .unwrap_or(&crate::chain::solana::TOKEN_PROGRAM_ID);

                // SOL transfers credit `sol_storage` PDA, not the SplitVault PDA; SPL uses vault-owned ATA.
                let is_match = details.payee == vault_pda
                    || details.payee == vault_sol_storage
                    || is_ata_of(
                        details.payee,
                        vault_pda,
                        &details.mint.unwrap_or_default(),
                        spl_token_prog,
                    );
                if is_match {
                    let token_mint = details.mint.unwrap_or_default();
                    let is_sol_sweep = details.mint.is_none();
                    let spl_mint_for_snap = if is_sol_sweep { None } else { Some(token_mint) };
                    let snap_token_program = if is_sol_sweep {
                        None
                    } else {
                        details.token_program
                    };

                    let mut skip_sweep = false;
                    match crate::vault_balance::fetch_universalsettle_vault_snapshot(
                        provider.rpc_client(),
                        us_config.program_id,
                        seller,
                        spl_mint_for_snap,
                        snap_token_program,
                    )
                    .await
                    {
                        Ok(snap) => {
                            if is_sol_sweep {
                                let min_lamports = crate::parameters::resolve_u64_sync(
                                    crate::parameters::PR402_SWEEP_MIN_SPENDABLE_LAMPORTS,
                                    crate::parameters::PR402_SWEEP_MIN_SPENDABLE_LAMPORTS,
                                    crate::parameters::DEFAULT_SWEEP_MIN_SPENDABLE_LAMPORTS,
                                );
                                if snap.spendable_lamports < min_lamports {
                                    tracing::info!(
                                        spendable_lamports = snap.spendable_lamports,
                                        min_lamports,
                                        seller = %seller,
                                        "skip UniversalSettle sweep: SOL vault below threshold"
                                    );
                                    skip_sweep = true;
                                }
                            } else {
                                let min_spl = crate::parameters::resolve_sweep_min_spl_raw_for_mint(
                                    &token_mint,
                                );
                                if snap.spl_amount_raw < min_spl {
                                    tracing::info!(
                                        spl_amount_raw = snap.spl_amount_raw,
                                        min_spl,
                                        mint = %token_mint,
                                        seller = %seller,
                                        "skip UniversalSettle sweep: SPL vault below threshold"
                                    );
                                    skip_sweep = true;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "vault snapshot failed before sweep; sending sweep anyway"
                            );
                        }
                    }

                    if !skip_sweep {
                        let sweep_ix =
                            crate::chain::solana_universalsettle::build_sweep_instruction(
                                us_config.program_id,
                                provider.pubkey(),
                                vault_pda,
                                seller,
                                fee_dest,
                                token_mint,
                                details.amount,
                                is_sol_sweep,
                                details.token_program,
                            );

                        let (recent_blockhash, _) = provider
                            .rpc_client()
                            .get_latest_blockhash_with_commitment(CommitmentConfig::confirmed())
                            .await?;
                        let sweep_tx = VersionedTransaction::from(
                            solana_transaction::Transaction::new_signed_with_payer(
                                &[sweep_ix],
                                Some(&provider.pubkey()),
                                &[provider.keypair()],
                                recent_blockhash,
                            ),
                        );
                        if let Err(e) = provider.send_sweep_transaction(&sweep_tx).await {
                            tracing::warn!(
                                error = %e,
                                payment_signature = %tx_sig,
                                seller = %seller,
                                "UniversalSettle sweep transaction send failed"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    payment_signature = %tx_sig,
                    "could not extract transfer from payment tx; UniversalSettle sweep skipped"
                );
            }
        }
    }

    Ok(tx_sig)
}

#[derive(Debug, thiserror::Error)]
pub enum SolanaExactError {
    #[error("Can not decode transaction: {0}")]
    TransactionDecoding(String),
    #[error("Compute unit limit exceeds facilitator maximum")]
    MaxComputeUnitLimitExceeded,
    #[error("Compute unit price exceeds facilitator maximum")]
    MaxComputeUnitPriceExceeded,
    #[error("Invalid transaction instructions count")]
    InvalidTransactionInstructionsCount,
    #[error("Fee payer included in instruction accounts")]
    FeePayerIncludedInInstructionAccounts,
    #[error("Fee payer found transferring funds")]
    FeePayerTransferringFunds,
    #[error("Instruction at index {0} not found")]
    NoInstructionAtIndex(usize),
    #[error("No account at index {0}")]
    NoAccountAtIndex(u8),
    #[error("Empty instruction at index {0}")]
    EmptyInstructionAtIndex(usize),
    #[error("Invalid compute limit instruction")]
    InvalidComputeLimitInstruction,
    #[error("Invalid compute price instruction")]
    InvalidComputePriceInstruction,
    #[error("Invalid Create ATA instruction")]
    InvalidCreateATAInstruction,
    #[error("Invalid token instruction")]
    InvalidTokenInstruction,
    #[error("UniversalSettle not fully configured")]
    UniversalSettleNotConfigured,
    #[error("Missing sender account in transaction")]
    MissingSenderAccount,
}

impl From<SolanaExactError> for PaymentVerificationError {
    fn from(e: SolanaExactError) -> Self {
        match e {
            SolanaExactError::TransactionDecoding(_) => {
                PaymentVerificationError::InvalidFormat(e.to_string())
            }
            _ => PaymentVerificationError::TransactionSimulation(e.to_string()),
        }
    }
}

impl From<SolanaChainProviderError> for PaymentVerificationError {
    fn from(value: SolanaChainProviderError) -> Self {
        Self::TransactionSimulation(value.to_string())
    }
}
