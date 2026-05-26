//! Per-transaction compute budget profiles for the pr402 Facilitator.
//!
//! Each [`TxBudget`] variant encodes the compute unit **limit** and micro-lamport
//! **price** appropriate for a specific on-chain operation type. Call
//! [`.cu_limit()`](TxBudget::cu_limit) and [`.cu_price()`](TxBudget::cu_price)
//! at build time to obtain the two `SetComputeUnit*` instruction arguments.
//!
//! # Values
//! Limits are set ~1.5–2× above observed peak CU consumption to absorb slot
//! congestion variance while staying far below the previous flat 400 k ceiling.
//! Prices reflect urgency: buyer-facing hot paths compete on mainnet; background
//! facilitator ops (vault creation, sweeps) use a lower price.
//!
//! Update values here as on-chain profiling data is collected — the enum is the
//! single source of truth for all budget decisions in pr402.

/// Per-transaction-type compute budget profile.
#[derive(Debug, Clone, Copy)]
pub enum TxBudget {
    // ── exact scheme (UniversalSettle) ───────────────────────────────
    /// `SystemProgram::Transfer` — native SOL payment.
    /// Facilitator is fee payer (slot 0); buyer signs transfer authority.
    /// Observed: ~300 CUs.
    ExactSolTransfer,

    /// `SPL TransferChecked` — USDC / USDT payment (any Token / Token-2022 SPL mint).
    /// Facilitator is fee payer (slot 0); buyer signs transfer authority.
    /// Observed: ~6 500 CUs.
    ExactSplTransfer,

    // ── facilitator JIT provisioning (ensure_vault_setup) ────────────
    /// `CreateVault` — SplitVault PDA + SOL-storage PDA initialisation.
    /// Facilitator signs and broadcasts immediately; includes two account writes.
    /// Estimated: ~40 000 CUs.
    VaultCreate,

    /// `CreateVault` + `ATA CreateIdempotent` — combined first-time SPL provisioning.
    /// Happens when a seller provisions a new SPL asset (e.g. USDC) for the first time:
    /// both the SplitVault PDA and the vault ATA are absent and are created in one tx.
    /// Estimated: ~40 000 + ~22 000 = ~62 000 CUs; limit set with headroom.
    VaultCreateWithAta,

    /// `ATA CreateIdempotent` — vault ATA creation for an SPL mint.
    /// Facilitator signs and broadcasts immediately.
    /// Observed: ~21 837 CUs.
    VaultAtaCreate,

    /// `CreateVault` + `ATA` + `Sweep` — the "Shadow Provision" case.
    /// Happens during settlement when a merchant hasn't provisioned yet.
    /// Includes multiple account creations and the sweep logic.
    /// Estimated: ~100 000+ CUs.
    VaultShadowProvision,

    // ── sweep cron (facilitator signs & broadcasts) ──────────────────
    /// `Sweep SOL` — 2× `SystemProgram::Transfer` (payout + protocol fee).
    /// Observed: ~12 868 CUs.
    SweepSol,

    /// `Sweep SPL` — 2× `SPL TransferChecked` (payout + protocol fee).
    /// Estimated: ~15 000 CUs (slightly heavier than SOL path).
    SweepSpl,

    // ── sla-escrow scheme ────────────────────────────────────────────
    /// `FundPayment` — escrow ATA creation (idempotent) + FundPayment instruction.
    /// Buyer pays now; facilitator may sponsor in future (`facilitatorPaysTransactionFees`).
    /// Estimated: ~80 000 CUs (account creation + token CPI).
    FundPayment,

    /// `ConfirmOracle` — lightweight state mutation; Oracle is the fee payer.
    /// No account creation involved.
    /// Estimated: ~15 000 CUs.
    OracleConfirm,

    /// `ReleasePayment` / `RefundPayment` — permissionless escrow settlement.
    /// SPL path may CPI-create seller/buyer/oracle ATAs on first release.
    /// Observed: >22_000 CUs when seller ATA is absent (settlement keeper cron).
    EscrowSettle,

    // ── fallback ─────────────────────────────────────────────────────
    /// Conservative fallback for future or unlisted instruction types.
    /// Use this when no specific variant applies yet.
    Default,
}

impl TxBudget {
    /// Compute unit limit to set via `SetComputeUnitLimit`.
    pub const fn cu_limit(self) -> u32 {
        match self {
            Self::ExactSolTransfer => 5_000,
            Self::ExactSplTransfer => 12_000,
            Self::VaultCreate => 55_000,
            Self::VaultCreateWithAta => 90_000,
            Self::VaultAtaCreate => 30_000,
            Self::VaultShadowProvision => 150_000,
            Self::SweepSol => 20_000,
            Self::SweepSpl => 22_000,
            Self::FundPayment => 80_000,
            Self::OracleConfirm => 20_000,
            Self::EscrowSettle => 80_000,
            Self::Default => 200_000,
        }
    }

    /// Micro-lamport price per compute unit, set via `SetComputeUnitPrice`.
    ///
    /// Hot buyer-facing paths use a higher price to compete on mainnet.
    /// Background facilitator operations (provisioning, sweeps) use a lower
    /// price since they are not latency-sensitive.
    pub const fn cu_price(self) -> u64 {
        match self {
            // Hot paths — buyer-facing, compete for block inclusion
            Self::ExactSolTransfer => 100_000,
            Self::ExactSplTransfer => 100_000,
            Self::FundPayment => 100_000,
            // Oracle confirm — still timely but oracle pays its own fees
            Self::OracleConfirm => 50_000,
            // Background facilitator ops — low urgency
            Self::VaultCreate => 10_000,
            Self::VaultCreateWithAta => 10_000,
            Self::VaultAtaCreate => 10_000,
            Self::VaultShadowProvision => 10_000,
            Self::SweepSol => 10_000,
            Self::SweepSpl => 10_000,
            Self::EscrowSettle => 10_000,
            // Fallback — conservative
            Self::Default => 100_000,
        }
    }
}
