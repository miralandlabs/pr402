#!/usr/bin/env node
/**
 * pr402-buy — one-shot buyer CLI (TypeScript).
 *
 * Runs the full x402 lifecycle against any seller URL: fetch 402 → build → sign →
 * verify → settle → retry. Seller-agnostic; uses the same `X402AgentClient` the
 * library exposes so the CLI and the importable API evolve together.
 *
 * Distribution:
 *   - Installed via the published npm package (`npm i -g @miraland-labs/pr402-client`),
 *     then: `pr402-buy --resource <url> --payer ~/.config/solana/id.json --mint <mint>`.
 *   - Or one-shot without installing: `npx @miraland-labs/pr402-client pr402-buy ...`.
 *   - No Rust toolchain needed. Works anywhere Node ≥ 18 runs.
 *
 * Flags are intentionally a subset of the Rust `pr402-buy` binary so that scripts can
 * target either implementation interchangeably; the underlying behavior is identical.
 */
export {};
