import { Keypair } from '@solana/web3.js';
/**
 * Specific, actionable error codes for autonomous agent remediation.
 *
 * @example
 * ```ts
 * try { await client.fetchWithAutoPay(url, mint); }
 * catch (e) {
 *   if (e instanceof X402Error && e.code === 'MINT_NOT_ACCEPTED')
 *     console.log('Available mints:', e.availableMints);
 * }
 * ```
 */
export type X402ErrorCode = 'UNEXPECTED_STATUS' | 'MISSING_ACCEPTS' | 'MINT_NOT_ACCEPTED' | 'MISSING_CAPABILITIES_URL' | 'BUILD_FAILED' | 'MISSING_VERIFY_TEMPLATE' | 'MISSING_TRANSACTION' | 'BLOCKHASH_EXPIRED' | 'RATE_LIMITED' | 'TRANSPORT';
export declare class X402Error extends Error {
    readonly code: X402ErrorCode;
    /** Mints accepted by the resource (only for MINT_NOT_ACCEPTED). */
    readonly availableMints?: string[];
    /** HTTP status from the facilitator (only for BUILD_FAILED / UNEXPECTED_STATUS). */
    readonly httpStatus?: number;
    /** Seconds to wait before retrying (only for RATE_LIMITED). */
    readonly retryAfterSecs?: number;
    /** UNIX epoch when blockhash expires (only for BLOCKHASH_EXPIRED). */
    readonly expiresAt?: number;
    constructor(code: X402ErrorCode, message: string, extra?: {
        availableMints?: string[];
        httpStatus?: number;
        retryAfterSecs?: number;
        expiresAt?: number;
    });
}
export interface FetchAutoPayOptions extends RequestInit {
    /** If true, the facilitator SDK build step will inject wSOL wrapping instructions automatically. */
    autoWrapSol?: boolean;
}
/**
 * Lightweight pr402 agent client.
 *
 * Wraps standard `fetch()` to automatically detect `402 Payment Required`,
 * delegate transaction construction to the pr402 Facilitator,
 * sign locally with Ed25519, and retry the original request with proof.
 *
 * @example
 * ```ts
 * const client = new X402AgentClient(myKeypair);
 * const res = await client.fetchWithAutoPay(url, usdcMint);
 * const data = await res.json();
 * ```
 */
export declare class X402AgentClient {
    private wallet;
    constructor(wallet: Keypair);
    /**
     * GET a 402-gated resource. If challenged, automatically build, sign, and settle.
     *
     * On success (seller returns 200) the retry request carries the signed
     * `verifyBodyTemplate` as a **`PAYMENT-SIGNATURE`** header (x402 v2). The value
     * is base64(UTF-8 JSON). Sellers in this ecosystem accept either base64 or raw
     * JSON in that header; this client emits base64 for URL-safety.
     *
     * @param url        - The target API endpoint
     * @param preferredMint - Base58 mint address of the token you want to pay with
     * @param options    - Optional extra fetch options (headers, autoWrapSol, etc.)
     * @throws {X402Error} with a specific `code` for each failure mode
     */
    fetchWithAutoPay(url: string, preferredMint: string, options?: FetchAutoPayOptions): Promise<Response>;
}
