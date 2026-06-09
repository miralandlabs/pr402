//! Shared Solana verification and settlement logic for v2:solana:exact.

use std::mem::size_of;

use solana_client::rpc_response::UiTransactionError;
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ID as ComputeBudgetInstructionId;
use solana_message::compiled_instruction::CompiledInstruction;
use solana_pubkey::{pubkey, Pubkey};
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction::TransactionError;
use tracing::error;

use crate::chain::solana::{Address, SolanaChainProvider, SolanaChainProviderError};
use crate::chain::solana_universalsettle::{self, Config as OnchainUsConfig};
use crate::proto::PaymentVerificationError;
use crate::util::{
    decode_versioned_transaction_from_bincode, reject_versioned_tx_with_address_lookup_tables,
    Base64Bytes,
};

pub const ATA_PROGRAM_PUBKEY: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// Attempt to read the `seller` field from an on-chain SplitVault account.
///
/// Returns `Some(seller_pubkey)` if:
///   - The account exists on-chain
///   - It is owned by the UniversalSettle program
///   - Its data is exactly 8 (discriminator) + 56 (SplitVault) = 64 bytes
///   - The derived vault PDA from the extracted seller matches the queried address
///     (consistency check: prevents accepting a spoofed account at a non-PDA address)
///
/// Returns `None` if the account doesn't exist, isn't a vault, or fails validation.
/// This function is intentionally conservative — a `None` result falls through to
/// the legacy "treat payTo as merchant wallet" path.
async fn resolve_seller_from_vault_account(
    provider: &SolanaChainProvider,
    candidate_vault: &Pubkey,
) -> Option<Pubkey> {
    let us_config = provider.universalsettle()?;
    let program_id = us_config.program_id;

    // Query the account
    let account = provider
        .rpc_client()
        .get_account(candidate_vault)
        .await
        .ok()?;

    // Must be owned by the UniversalSettle program
    if account.owner != program_id {
        return None;
    }

    // SplitVault on-chain layout: 8-byte discriminator + 56-byte struct = 64 bytes
    const DISCRIMINATOR_LEN: usize = 8;
    const SPLIT_VAULT_SIZE: usize = size_of::<solana_universalsettle::SplitVault>();
    if account.data.len() != DISCRIMINATOR_LEN + SPLIT_VAULT_SIZE {
        return None;
    }

    // Parse the seller pubkey (first 32 bytes after discriminator)
    let vault_data = &account.data[DISCRIMINATOR_LEN..];
    let vault: &solana_universalsettle::SplitVault =
        bytemuck::from_bytes(&vault_data[..SPLIT_VAULT_SIZE]);
    let seller = vault.seller;

    // Consistency check: verify that find_program_address([b"vault", seller], program_id)
    // actually produces the address we queried. This prevents accepting a maliciously
    // placed account at an arbitrary address.
    let (expected_vault_pda, _) =
        Pubkey::find_program_address(&[b"vault", seller.as_ref()], &program_id);
    if expected_vault_pda != *candidate_vault {
        tracing::warn!(
            candidate = %candidate_vault,
            expected = %expected_vault_pda,
            seller = %seller,
            "resolve_seller_from_vault_account: PDA consistency check failed"
        );
        return None;
    }

    tracing::info!(
        vault = %candidate_vault,
        seller = %seller,
        "resolve_seller_from_vault_account: extracted seller from on-chain vault"
    );
    Some(seller)
}

#[derive(Clone, Debug)]
pub struct TransferRequirement<'a> {
    pub asset: &'a Address,
    pub pay_to: &'a Address,
    pub amount: u64,
    pub merchant_wallet: Option<&'a Address>, // IDENTITY: Original wallet for derivation
    pub collection_beneficiary: Option<&'a Address>, // COLLECTION: Priority payout destination
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
    pub merchant_identity: Address,
    pub final_beneficiary: Address,
    pub vault_pda: Pubkey,
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
    limit_ceiling: u32,
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

    if compute_units > limit_ceiling {
        return Err(SolanaExactError::MaxComputeUnitLimitExceeded);
    }
    Ok(compute_units)
}

pub fn verify_compute_price_instruction(
    price_ceiling: u64,
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
    if microlamports > price_ceiling {
        return Err(SolanaExactError::MaxComputeUnitPriceExceeded);
    }
    Ok(())
}

fn parse_set_compute_unit_limit(data: &[u8]) -> Option<u32> {
    if data.first()? != &2 || data.len() != 5 {
        return None;
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&data[1..5]);
    Some(u32::from_le_bytes(buf))
}

fn parse_set_compute_unit_price(data: &[u8]) -> Option<u64> {
    if data.first()? != &3 || data.len() != 9 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[1..9]);
    Some(u64::from_le_bytes(buf))
}

/// Solana uses the **last** `SetComputeUnitLimit` in the transaction. Wallets often
/// prepend their own compute-budget instructions ahead of pr402-built ones; verify the
/// effective limit rather than assuming it lives at index 0.
pub fn verify_effective_compute_unit_limit(
    limit_ceiling: u32,
    transaction: &VersionedTransaction,
) -> Result<u32, SolanaExactError> {
    let keys = transaction.message.static_account_keys();
    let mut effective: Option<u32> = None;
    for ix in transaction.message.instructions() {
        let program_id = ix.program_id(keys);
        if ComputeBudgetInstructionId.ne(program_id) {
            continue;
        }
        if let Some(units) = parse_set_compute_unit_limit(ix.data.as_slice()) {
            effective = Some(units);
        }
    }
    let units = effective.ok_or(SolanaExactError::InvalidComputeLimitInstruction)?;
    if units > limit_ceiling {
        return Err(SolanaExactError::MaxComputeUnitLimitExceeded);
    }
    Ok(units)
}

/// Same last-wins semantics as [`verify_effective_compute_unit_limit`].
pub fn verify_effective_compute_unit_price(
    price_ceiling: u64,
    transaction: &VersionedTransaction,
) -> Result<(), SolanaExactError> {
    let keys = transaction.message.static_account_keys();
    let mut effective: Option<u64> = None;
    for ix in transaction.message.instructions() {
        let program_id = ix.program_id(keys);
        if ComputeBudgetInstructionId.ne(program_id) {
            continue;
        }
        if let Some(price) = parse_set_compute_unit_price(ix.data.as_slice()) {
            effective = Some(price);
        }
    }
    let price = effective.ok_or(SolanaExactError::InvalidComputePriceInstruction)?;
    if price > price_ceiling {
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
    let transaction = decode_versioned_transaction_from_bincode(bytes.as_slice())
        .map_err(SolanaExactError::TransactionDecoding)?;
    reject_versioned_tx_with_address_lookup_tables(&transaction)
        .map_err(SolanaExactError::TransactionDecoding)?;

    let cfg = solana_client::rpc_config::RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: false,
        commitment: Some(solana_commitment_config::CommitmentConfig::confirmed()),
        encoding: None,
        accounts: None,
        inner_instructions: false,
        min_context_slot: None,
    };

    let sim_result = provider
        .simulate_transaction_with_config(&transaction, cfg)
        .await
        .map_err(|e| PaymentVerificationError::TransactionSimulation(e.to_string()))?;

    if let Some(err) = sim_result.value.err {
        return Err(PaymentVerificationError::TransactionSimulation(format!(
            "Simulation failed on-chain: {:?}",
            err
        )));
    }

    // perform transaction introspection to validate the transaction structure and details
    let instructions = transaction.message.instructions();
    let budget = crate::chain::TxBudget::ExactSplTransfer;
    let _compute_units = verify_compute_limit_instruction(budget.cu_limit(), &transaction, 0)?;
    verify_compute_price_instruction(budget.cu_price(), &transaction, 1)?;

    let transfer_instruction = match instructions.len() {
        3 => {
            verify_transfer_instruction(provider, &transaction, 2, transfer_requirement, false)
                .await?
        }
        4 => {
            verify_create_ata_instruction(&transaction, 2, transfer_requirement)?;
            verify_transfer_instruction(provider, &transaction, 3, transfer_requirement, true)
                .await?
        }
        6 => {
            // indices 2, 3, 4 are auto-wrap (create_ata, transfer SOL, sync_native)
            verify_transfer_instruction(provider, &transaction, 5, transfer_requirement, false)
                .await?
        }
        7 => {
            // indices 2, 3, 4 are auto-wrap, index 5 is dest create_ata
            verify_create_ata_instruction(&transaction, 5, transfer_requirement)?;
            verify_transfer_instruction(provider, &transaction, 6, transfer_requirement, true)
                .await?
        }
        _ => return Err(SolanaExactError::InvalidTransactionInstructionsCount.into()),
    };

    let pay_to_pk = *transfer_requirement.pay_to.pubkey();
    let (vault_from_pay_to, _) = provider.get_vault_pda(&pay_to_pk);

    // ADAPTIVE PDA RESOLUTION: resolve the authoritative merchant identity (wallet)
    // and the target vault PDA in a single pass.
    //
    // Three cases:
    //   1. `merchant_wallet` is explicitly provided in `accepts[].extra` → use it directly.
    //   2. `merchant_wallet` is absent, but `payTo` is an existing SplitVault PDA on-chain →
    //      read the `seller` field from the vault account data. This handles the common case
    //      where sellers set `payTo = vault_pda` (correct x402 semantics) without also
    //      providing `merchantWallet` in the extra.
    //   3. `merchant_wallet` is absent and `payTo` is NOT a vault PDA → treat `payTo` as the
    //      merchant wallet identity (backward-compatible for direct-wallet payTo).
    //
    // SAFETY: The on-chain `CreateVault` instruction validates that the vault PDA matches
    // `find_program_address([b"vault", seller], program_id)`. So reading `vault.seller` from
    // an account owned by the UniversalSettle program is authoritative — it cannot be spoofed.
    let (final_vault_pda, merchant_identity) =
        if let Some(identity) = transfer_requirement.merchant_wallet {
            // Case 1: explicit merchant_wallet in accepts[].extra
            let (v, _) = provider.get_vault_pda(identity.pubkey());
            (v, *identity.pubkey())
        } else if let Some(seller) = resolve_seller_from_vault_account(provider, &pay_to_pk).await {
            // Case 2: payTo is an existing SplitVault PDA — extract seller from on-chain data
            (pay_to_pk, seller)
        } else {
            // Case 3: payTo is the merchant wallet identity (not a vault PDA)
            (vault_from_pay_to, pay_to_pk)
        };

    // Rule: UniversalSettle SplitVault enforcement
    if provider.universalsettle().is_some() {
        let is_sol = *transfer_requirement.asset.pubkey() == Pubkey::default();
        let dest_match = if is_sol {
            let (vault_sol_storage, _) = provider.get_sol_storage_pda(final_vault_pda);
            transfer_instruction.destination == vault_sol_storage
        } else {
            is_ata_of(
                transfer_instruction.destination,
                final_vault_pda,
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
    // COLLECTION PRIORITY: beneficiary (from requirement) > merchant identity fallback
    let final_beneficiary = transfer_requirement
        .collection_beneficiary
        .cloned()
        .unwrap_or_else(|| Address::new(merchant_identity));

    Ok(VerifyTransferResult {
        payer,
        merchant_identity: Address::new(merchant_identity),
        final_beneficiary,
        vault_pda: final_vault_pda,
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
    } else if crate::chain::solana::TOKEN_2022_PROGRAM_ID.eq(&program_id) {
        // Same `TransferChecked` wire format as legacy Token; avoid `spl-token-2022` crate (see Cargo.toml).
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
            token_program: crate::chain::solana::TOKEN_2022_PROGRAM_ID,
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
    collection_beneficiary: Option<Pubkey>, // COLLECTION: Priority payout destination
    db: Option<&crate::db::Pr402Db>,        // SELL-3: reuse existing pool
) -> Result<Signature, SolanaChainProviderError> {
    let tx = TransactionInt::new(verification.transaction).sign(provider)?;
    if !tx.is_fully_signed() {
        return Err(SolanaChainProviderError::InvalidTransaction(
            UiTransactionError::from(TransactionError::SignatureFailure),
        ));
    }
    let primary = tx.inner.signatures[0];

    match provider
        .rpc_client()
        .get_signature_status_with_commitment(&primary, CommitmentConfig::confirmed())
        .await
    {
        Ok(Some(Ok(()))) => return Ok(primary),
        Ok(Some(Err(e))) => {
            return Err(SolanaChainProviderError::Transport(format!(
                "transaction failed on-chain: {:?}",
                e
            )));
        }
        Ok(None) | Err(_) => {}
    }

    let tx_sig = match tx
        .send_and_confirm(provider, CommitmentConfig::confirmed())
        .await
    {
        Ok(sig) => sig,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Blockhash not found")
                || msg.contains("blockhash not found")
                || msg.contains("BlockhashNotFound")
            {
                return Err(SolanaChainProviderError::Transport(
                    "retry build: transaction blockhash has expired or is invalid".to_string(),
                ));
            }
            if msg.contains("already been processed") || msg.contains("AlreadyProcessed") {
                if matches!(
                    provider
                        .rpc_client()
                        .get_signature_status_with_commitment(
                            &primary,
                            CommitmentConfig::confirmed()
                        )
                        .await,
                    Ok(Some(Ok(())))
                ) {
                    primary
                } else {
                    return Err(e);
                }
            } else {
                return Err(e);
            }
        }
    };

    if let Some(us_config) = provider.universalsettle() {
        let payer = *verification.payer.pubkey();
        match provider.extract_transfer_from_pst(&tx.inner, &payer) {
            Ok(details) => {
                let merchant_identity = *verification.merchant_identity.pubkey();
                let vault_pda = verification.vault_pda;
                let (vault_sol_storage, _) = provider.get_sol_storage_pda(vault_pda);

                // COLLECTION PRIORITY: prioritized final_beneficiary from verification (or override)
                let final_beneficiary =
                    collection_beneficiary.unwrap_or(*verification.final_beneficiary.pubkey());
                let fee_dest = match us_config.fee_destination {
                    Some(dest) => dest,
                    None => {
                        error!(
                            merchant = %merchant_identity,
                            "UniversalSettle fee destination NOT CONFIGURED: skipping sweep"
                        );
                        return Ok(tx_sig);
                    }
                };

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
                        merchant_identity,
                        spl_mint_for_snap,
                        snap_token_program,
                    )
                    .await
                    {
                        Ok(snap) => {
                            let global_floor = if is_sol_sweep {
                                crate::parameters::resolve_u64_sync(
                                    crate::parameters::PR402_SWEEP_MIN_SPENDABLE_LAMPORTS,
                                    crate::parameters::PR402_SWEEP_MIN_SPENDABLE_LAMPORTS,
                                    crate::parameters::DEFAULT_SWEEP_MIN_SPENDABLE_LAMPORTS,
                                )
                            } else {
                                crate::parameters::resolve_sweep_min_spl_raw_for_mint(&token_mint)
                            };

                            let merchant_floor = if let Some(pool) = db {
                                let spl_mint_for_db = spl_mint_for_snap.map(|m| m.to_string());
                                pool.get_resource_provider_sweep_threshold(
                                    &merchant_identity.to_string(),
                                    spl_mint_for_db.as_deref(),
                                )
                                .await
                                .unwrap_or(None)
                            } else {
                                None
                            };

                            let safe_sweep_threshold =
                                std::cmp::max(merchant_floor.unwrap_or(0), global_floor);

                            if is_sol_sweep {
                                if snap.spendable_lamports < safe_sweep_threshold {
                                    tracing::info!(
                                        spendable_lamports = snap.spendable_lamports,
                                        safe_sweep_threshold,
                                        seller = %merchant_identity,
                                        "skip UniversalSettle sweep: SOL vault below safe threshold"
                                    );
                                    skip_sweep = true;
                                }
                            } else if snap.spl_amount_raw < safe_sweep_threshold {
                                tracing::info!(
                                    spl_amount_raw = snap.spl_amount_raw,
                                    safe_sweep_threshold,
                                    mint = %token_mint,
                                    seller = %merchant_identity,
                                    "skip UniversalSettle sweep: SPL vault below safe threshold"
                                );
                                skip_sweep = true;
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
                        let mut instructions = Vec::new();

                        // SE-CRIT-11: JIT Onboarding logic.
                        let mut vault_missing = false;
                        match provider.account_exists(&vault_pda).await {
                            Ok(exists) => vault_missing = !exists,
                            Err(e) => {
                                tracing::warn!(error = %e, "vault existence check failed during settlement");
                            }
                        }

                        if vault_missing {
                            let max_provisions = crate::parameters::resolve_u64_sync(
                                crate::parameters::PR402_MAX_DAILY_PROVISION_COUNT,
                                crate::parameters::PR402_MAX_DAILY_PROVISION_COUNT,
                                crate::parameters::DEFAULT_MAX_DAILY_PROVISION_COUNT,
                            );
                            let count = match db {
                                Some(pool) => pool.count_daily_vault_creations().await.unwrap_or(0),
                                None => 0,
                            };

                            if count < max_provisions {
                                tracing::info!(
                                    seller = %merchant_identity,
                                    vault = %vault_pda,
                                    "provisioning new SplitVault (JIT shadow provision)"
                                );
                                instructions.push(crate::chain::solana_universalsettle::build_create_vault_instruction(
                                    us_config.program_id,
                                    provider.pubkey(),
                                    merchant_identity,
                                ));
                            } else {
                                tracing::warn!(
                                    seller = %merchant_identity,
                                    "skip UniversalSettle shadow provision: daily quota reached"
                                );
                                skip_sweep = true;
                            }
                        }

                        if !skip_sweep {
                            // SPL case: Ensure Vault ATA exists (JIT ATA shadow provision)
                            if let Some(token_program) = details.token_program {
                                let (ata, _) = Pubkey::find_program_address(
                                    &[
                                        vault_pda.as_ref(),
                                        token_program.as_ref(),
                                        token_mint.as_ref(),
                                    ],
                                    &crate::chain::solana::ASSOCIATED_TOKEN_PROGRAM_ID,
                                );
                                match provider.account_exists(&ata).await {
                                    Ok(exists) if !exists => {
                                        tracing::info!(vault = %vault_pda, mint = %token_mint, "creating vault ATA (JIT shadow provision)");
                                        instructions.push(crate::util::tx_builder::create_associated_token_account_idempotent_ix(
                                            &provider.pubkey(),
                                            &vault_pda,
                                            &token_mint,
                                            &token_program,
                                        ));
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "vault ATA existence check failed during settlement");
                                    }
                                    _ => {}
                                }
                            }

                            let (fee_sol_recv, fee_token_owner) = match provider
                                .rpc_client()
                                .get_account(&provider.get_config_pda(&us_config.program_id).0)
                                .await
                            {
                                Ok(acc) if acc.data.len() >= 8 + size_of::<OnchainUsConfig>() => {
                                    let cfg: &OnchainUsConfig = bytemuck::from_bytes(
                                        &acc.data[8..8 + size_of::<OnchainUsConfig>()],
                                    );
                                    if cfg.fee_destination != fee_dest {
                                        tracing::warn!(
                                            configured_fee_dest = %fee_dest,
                                            chain_fee_dest = %cfg.fee_destination,
                                            "UniversalSettle sweep: env fee_destination differs from on-chain config.fee_destination; routing fees using on-chain treasury + shard flags"
                                        );
                                    }
                                    solana_universalsettle::sweep_fee_destinations(
                                        &us_config.program_id,
                                        &cfg.fee_destination,
                                        &merchant_identity,
                                        cfg.use_fee_shard,
                                        cfg.shard_count,
                                    )
                                }
                                Ok(_) | Err(_) => {
                                    tracing::warn!(
                                        "UniversalSettle sweep: could not load config PDA; defaulting fee leg to configured fee_destination"
                                    );
                                    (fee_dest, fee_dest)
                                }
                            };

                            if let Some(token_program) = details.token_program {
                                let beneficiary_ata =
                                    crate::util::tx_builder::associated_token_address(
                                        &final_beneficiary,
                                        &token_mint,
                                        &token_program,
                                    );
                                match provider.account_exists(&beneficiary_ata).await {
                                    Ok(true) => {}
                                    Ok(false) => {
                                        instructions.push(crate::util::tx_builder::create_associated_token_account_idempotent_ix(
                                            &provider.pubkey(),
                                            &final_beneficiary,
                                            &token_mint,
                                            &token_program,
                                        ));
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            error = %e,
                                            ata = %beneficiary_ata,
                                            "beneficiary ATA existence check failed before inline sweep; adding idempotent create instruction"
                                        );
                                        instructions.push(crate::util::tx_builder::create_associated_token_account_idempotent_ix(
                                            &provider.pubkey(),
                                            &final_beneficiary,
                                            &token_mint,
                                            &token_program,
                                        ));
                                    }
                                }
                                if fee_token_owner != final_beneficiary {
                                    let fee_ata = crate::util::tx_builder::associated_token_address(
                                        &fee_token_owner,
                                        &token_mint,
                                        &token_program,
                                    );
                                    match provider.account_exists(&fee_ata).await {
                                        Ok(true) => {}
                                        Ok(false) => {
                                            instructions.push(crate::util::tx_builder::create_associated_token_account_idempotent_ix(
                                                &provider.pubkey(),
                                                &fee_token_owner,
                                                &token_mint,
                                                &token_program,
                                            ));
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                ata = %fee_ata,
                                                "fee ATA existence check failed before inline sweep; adding idempotent create instruction"
                                            );
                                            instructions.push(crate::util::tx_builder::create_associated_token_account_idempotent_ix(
                                                &provider.pubkey(),
                                                &fee_token_owner,
                                                &token_mint,
                                                &token_program,
                                            ));
                                        }
                                    }
                                }
                            }

                            instructions.push(
                                crate::chain::solana_universalsettle::build_sweep_instruction(
                                    us_config.program_id,
                                    provider.pubkey(),
                                    vault_pda,
                                    final_beneficiary,
                                    fee_sol_recv,
                                    fee_token_owner,
                                    token_mint,
                                    0,
                                    is_sol_sweep,
                                    details.token_program,
                                ),
                            );

                            // Determine Budget Profile
                            // We have at least Sweep. Maybe CreateVault, maybe ATA.
                            let budget = if instructions.iter().any(|ix| {
                                ix.data.first() == Some(&1) && ix.program_id == us_config.program_id
                            }) {
                                // Has CreateVault (discriminator 1)
                                crate::chain::TxBudget::VaultShadowProvision
                            } else if instructions.len() > 1 {
                                // Has ATA + Sweep
                                crate::chain::TxBudget::SweepSplWithAta
                            } else if is_sol_sweep {
                                crate::chain::TxBudget::SweepSol
                            } else {
                                crate::chain::TxBudget::SweepSpl
                            };

                            let cu_limit = crate::util::tx_builder::compute_budget_ix_set_limit(
                                budget.cu_limit(),
                            );
                            let cu_price = crate::util::tx_builder::compute_budget_ix_set_price(
                                budget.cu_price(),
                            );
                            let mut final_ixs = vec![cu_limit, cu_price];
                            final_ixs.extend(instructions);

                            let (recent_blockhash, _) = provider
                                .rpc_client()
                                .get_latest_blockhash_with_commitment(CommitmentConfig::confirmed())
                                .await?;
                            let sweep_tx = VersionedTransaction::from(
                                solana_transaction::Transaction::new_signed_with_payer(
                                    &final_ixs,
                                    Some(&provider.pubkey()),
                                    &[provider.keypair()],
                                    recent_blockhash,
                                ),
                            );
                            if let Err(e) = provider.send_sweep_transaction(&sweep_tx).await {
                                tracing::warn!(
                                    error = %e,
                                    payment_signature = %tx_sig,
                                    seller = %merchant_identity,
                                    "UniversalSettle sweep transaction send failed"
                                );
                            }
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

#[cfg(test)]
mod compute_budget_tests {
    use super::*;
    use crate::chain::TxBudget;
    use crate::util::tx_builder::{compute_budget_ix_set_limit, compute_budget_ix_set_price};
    use solana_hash::Hash;
    use solana_keypair::Keypair;
    use solana_signer::Signer;
    use solana_transaction::{Instruction, Transaction};

    fn tx_with_budget_ixs(ixs: Vec<Instruction>) -> VersionedTransaction {
        let payer = Keypair::new();
        VersionedTransaction::from(Transaction::new_signed_with_payer(
            &ixs,
            Some(&payer.pubkey()),
            &[&payer],
            Hash::new_unique(),
        ))
    }

    #[test]
    fn effective_cu_uses_last_limit_and_price() {
        let budget = TxBudget::FundPayment;
        let tx = tx_with_budget_ixs(vec![
            compute_budget_ix_set_limit(400_000),
            compute_budget_ix_set_price(budget.cu_price()),
            compute_budget_ix_set_limit(budget.cu_limit()),
            compute_budget_ix_set_price(budget.cu_price()),
        ]);

        assert_eq!(
            verify_effective_compute_unit_limit(budget.cu_limit(), &tx).unwrap(),
            budget.cu_limit()
        );
        assert!(verify_effective_compute_unit_price(budget.cu_price(), &tx).is_ok());
    }

    #[test]
    fn effective_cu_rejects_limit_above_ceiling() {
        let budget = TxBudget::FundPayment;
        let tx = tx_with_budget_ixs(vec![
            compute_budget_ix_set_limit(budget.cu_limit()),
            compute_budget_ix_set_limit(budget.cu_limit() + 1),
            compute_budget_ix_set_price(budget.cu_price()),
        ]);

        assert!(matches!(
            verify_effective_compute_unit_limit(budget.cu_limit(), &tx),
            Err(SolanaExactError::MaxComputeUnitLimitExceeded)
        ));
    }

    #[test]
    fn effective_cu_requires_price_instruction() {
        let budget = TxBudget::FundPayment;
        let tx = tx_with_budget_ixs(vec![compute_budget_ix_set_limit(budget.cu_limit())]);

        assert!(matches!(
            verify_effective_compute_unit_price(budget.cu_price(), &tx),
            Err(SolanaExactError::InvalidComputePriceInstruction)
        ));
    }

    #[test]
    fn effective_cu_requires_limit_instruction() {
        let budget = TxBudget::FundPayment;
        let tx = tx_with_budget_ixs(vec![compute_budget_ix_set_price(budget.cu_price())]);

        assert!(matches!(
            verify_effective_compute_unit_limit(budget.cu_limit(), &tx),
            Err(SolanaExactError::InvalidComputeLimitInstruction)
        ));
    }

    #[test]
    fn effective_cu_allows_budget_and_ata_prefix_before_fund_path() {
        let budget = TxBudget::FundPayment;
        let tx = tx_with_budget_ixs(vec![
            compute_budget_ix_set_limit(400_000),
            compute_budget_ix_set_price(budget.cu_price()),
            compute_budget_ix_set_limit(budget.cu_limit()),
            compute_budget_ix_set_price(budget.cu_price()),
            Instruction {
                program_id: ATA_PROGRAM_PUBKEY,
                accounts: vec![],
                data: vec![1],
            },
        ]);

        assert_eq!(
            verify_effective_compute_unit_limit(budget.cu_limit(), &tx).unwrap(),
            budget.cu_limit()
        );
        assert!(verify_effective_compute_unit_price(budget.cu_price(), &tx).is_ok());
    }
}
