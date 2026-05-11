#!/usr/bin/env node
/**
 * pr402-buy — one-shot buyer CLI (TypeScript).
 *
 * Runs the full x402 lifecycle against any seller URL: fetch 402 → build → sign →
 * verify → settle → retry. Seller-agnostic; uses the same `X402AgentClient` the
 * library exposes so the CLI and the importable API evolve together.
 *
 * Distribution:
 *   - Installed via the published npm package (`npm i -g @pr402/client`),
 *     then: `pr402-buy --resource <url> --payer ~/.config/solana/id.json --mint <mint>`.
 *   - Or one-shot without installing: `npx @pr402/client pr402-buy ...`.
 *   - No Rust toolchain needed. Works anywhere Node ≥ 18 runs.
 *
 * Flags are intentionally a subset of the Rust `pr402-buy` binary so that scripts can
 * target either implementation interchangeably; the underlying behavior is identical.
 */

import * as fs from "node:fs";
import { Keypair } from "@solana/web3.js";
import { X402AgentClient, X402Error } from "./index.js";

interface Args {
    resource: string;
    payer: string;
    mint: string;
    verbose: boolean;
    autoWrapSol: boolean;
    help: boolean;
}

const USAGE = `pr402-buy — one-shot buyer for x402 v2 resources.

Usage:
  pr402-buy --resource <URL> --payer <KEYPAIR_PATH> --mint <MINT>

Options:
  --resource, -r  <URL>          Seller resource URL. GET first; if 200, done. If 402, pay + retry.
  --payer,    -p  <PATH>         Path to a Solana keypair JSON (array of 64 bytes, same as solana-keygen output).
  --mint,     -m  <PUBKEY>       Base58 mint to pay with. Picks the matching accepts[] line from the 402 body.
  --auto-wrap-sol                Ask the facilitator to wrap SOL automatically when paying with WSOL.
  --verbose,  -v                 Print bodies at each step.
  --help,     -h                 This help.

Exit codes:
  0  resource fetched successfully
  1  usage / flag error
  2  network or HTTP transport failure
  3  protocol-level failure (facilitator / seller rejected the flow)
`;

function parseArgs(argv: string[]): Args {
    const out: Args = {
        resource: "",
        payer: "",
        mint: "",
        verbose: false,
        autoWrapSol: false,
        help: false,
    };
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        const next = () => argv[++i] ?? "";
        switch (a) {
            case "--resource":
            case "-r":
                out.resource = next();
                break;
            case "--payer":
            case "-p":
                out.payer = next();
                break;
            case "--mint":
            case "-m":
                out.mint = next();
                break;
            case "--auto-wrap-sol":
                out.autoWrapSol = true;
                break;
            case "--verbose":
            case "-v":
                out.verbose = true;
                break;
            case "--help":
            case "-h":
                out.help = true;
                break;
            default:
                // Unknown flag: bail early so typos don't silently succeed with defaults.
                throw new Error(`unknown flag: ${a}`);
        }
    }
    return out;
}

function loadKeypair(path: string): Keypair {
    // Solana CLI keypair format: JSON array of 64 bytes.
    const raw = fs.readFileSync(path, "utf8");
    const bytes = JSON.parse(raw);
    if (!Array.isArray(bytes) || bytes.length !== 64) {
        throw new Error(
            `keypair file ${path} must be a JSON array of 64 bytes, got ${
                Array.isArray(bytes) ? `array of ${bytes.length}` : typeof bytes
            }`,
        );
    }
    return Keypair.fromSecretKey(new Uint8Array(bytes));
}

async function main(): Promise<number> {
    let args: Args;
    try {
        args = parseArgs(process.argv.slice(2));
    } catch (e) {
        process.stderr.write(`${(e as Error).message}\n\n${USAGE}`);
        return 1;
    }

    if (args.help) {
        process.stdout.write(USAGE);
        return 0;
    }
    if (!args.resource || !args.payer || !args.mint) {
        process.stderr.write(`missing required flag.\n\n${USAGE}`);
        return 1;
    }

    const payer = loadKeypair(args.payer);
    if (args.verbose) {
        process.stderr.write(`payer: ${payer.publicKey.toBase58()}\n`);
    }

    const client = new X402AgentClient(payer);
    try {
        const res = await client.fetchWithAutoPay(args.resource, args.mint, {
            autoWrapSol: args.autoWrapSol,
        });
        const text = await res.text();
        if (!res.ok) {
            process.stderr.write(
                `resource retry failed (HTTP ${res.status}): ${text}\n`,
            );
            return 3;
        }
        const paymentResponse = res.headers.get("PAYMENT-RESPONSE");
        if (args.verbose && paymentResponse) {
            process.stderr.write(
                `PAYMENT-RESPONSE (base64): ${paymentResponse}\n`,
            );
        }
        process.stdout.write(text);
        // Newline only when stdout is a TTY so piped output stays byte-identical.
        if (process.stdout.isTTY) process.stdout.write("\n");
        return 0;
    } catch (e) {
        if (e instanceof X402Error) {
            // Protocol-level error codes come with actionable context — surface them.
            process.stderr.write(`${e.code}: ${e.message}\n`);
            if (e.availableMints?.length) {
                process.stderr.write(
                    `available mints: ${e.availableMints.join(", ")}\n`,
                );
            }
            if (e.retryAfterSecs) {
                process.stderr.write(`retry after: ${e.retryAfterSecs}s\n`);
            }
            if (e.expiresAt) {
                process.stderr.write(`blockhash expired at unix ${e.expiresAt}\n`);
            }
            return 3;
        }
        process.stderr.write(`transport error: ${(e as Error).message}\n`);
        return 2;
    }
}

main().then(
    (code) => process.exit(code),
    (e) => {
        process.stderr.write(`unexpected: ${(e as Error).stack ?? e}\n`);
        process.exit(2);
    },
);
