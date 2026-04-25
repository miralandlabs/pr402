import { Keypair, VersionedTransaction } from '@solana/web3.js';

// ── Error types ─────────────────────────────────────────────────────────

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
export type X402ErrorCode =
  | 'UNEXPECTED_STATUS'
  | 'MISSING_ACCEPTS'
  | 'MINT_NOT_ACCEPTED'
  | 'MISSING_CAPABILITIES_URL'
  | 'BUILD_FAILED'
  | 'MISSING_VERIFY_TEMPLATE'
  | 'MISSING_TRANSACTION'
  | 'BLOCKHASH_EXPIRED'
  | 'RATE_LIMITED'
  | 'TRANSPORT';

export class X402Error extends Error {
  readonly code: X402ErrorCode;
  /** Mints accepted by the resource (only for MINT_NOT_ACCEPTED). */
  readonly availableMints?: string[];
  /** HTTP status from the facilitator (only for BUILD_FAILED / UNEXPECTED_STATUS). */
  readonly httpStatus?: number;
  /** Seconds to wait before retrying (only for RATE_LIMITED). */
  readonly retryAfterSecs?: number;
  /** UNIX epoch when blockhash expires (only for BLOCKHASH_EXPIRED). */
  readonly expiresAt?: number;

  constructor(
    code: X402ErrorCode,
    message: string,
    extra?: {
      availableMints?: string[];
      httpStatus?: number;
      retryAfterSecs?: number;
      expiresAt?: number;
    }
  ) {
    super(message);
    this.name = 'X402Error';
    this.code = code;
    this.availableMints = extra?.availableMints;
    this.httpStatus = extra?.httpStatus;
    this.retryAfterSecs = extra?.retryAfterSecs;
    this.expiresAt = extra?.expiresAt;
  }
}

// ── Client ──────────────────────────────────────────────────────────────

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
export class X402AgentClient {
  private wallet: Keypair;

  constructor(wallet: Keypair) {
    this.wallet = wallet;
  }

  /**
   * GET a 402-gated resource. If challenged, automatically build, sign, and settle.
   *
   * @param url        - The target API endpoint
   * @param preferredMint - Base58 mint address of the token you want to pay with
   * @param options    - Optional extra fetch options (headers, autoWrapSol, etc.)
   * @throws {X402Error} with a specific `code` for each failure mode
   */
  async fetchWithAutoPay(
    url: string,
    preferredMint: string,
    options?: FetchAutoPayOptions
  ): Promise<Response> {
    const res = await fetch(url, options);

    if (res.status === 200) return res;
    if (res.status !== 402)
      throw new X402Error(
        'UNEXPECTED_STATUS',
        `Unexpected HTTP status ${res.status}. Expected 200 (free) or 402 (payment required).`,
        { httpStatus: res.status }
      );

    // ── Step 1: Parse the 402 Challenge ─────────────────────────────
    const requirement = await res.json();
    const accepts: any[] = requirement.accepts || [];

    if (accepts.length === 0)
      throw new X402Error(
        'MISSING_ACCEPTS',
        "The 402 response has no 'accepts' array. The Resource Provider's payment configuration is invalid. Contact the RP operator."
      );

    const availableMints = accepts
      .map((a: any) => a.asset as string)
      .filter(Boolean);

    const rule = accepts.find((a: any) => a.asset === preferredMint);
    if (!rule)
      throw new X402Error(
        'MINT_NOT_ACCEPTED',
        `Resource does not accept mint ${preferredMint}. Available mints: [${availableMints.join(', ')}]. Pick one from this list.`,
        { availableMints }
      );

    const capUrl: string | undefined = rule.extra?.capabilitiesUrl;
    if (!capUrl)
      throw new X402Error(
        'MISSING_CAPABILITIES_URL',
        'This 402-gated resource did not provide extra.capabilitiesUrl. The Resource Provider has not completed Facilitator integration. See docs/SELLER_INTEGRATION.md.'
      );

    // ── Step 2: Ask Facilitator to build the tx ─────────────────────
    const facilitatorBase = capUrl.replace('/capabilities', '');
    const buildRes = await fetch(
      `${facilitatorBase}/build-exact-payment-tx`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          payer: this.wallet.publicKey.toBase58(),
          accepted: rule,
          resource: requirement.resource,
          skipSourceBalanceCheck: true,
          autoWrapSol: options?.autoWrapSol,
        }),
      }
    );

    if (buildRes.status === 429) {
      const retryAfter = parseInt(
        buildRes.headers.get('retry-after') || '60',
        10
      );
      throw new X402Error(
        'RATE_LIMITED',
        `Facilitator rate-limited this request. Retry after ${retryAfter}s.`,
        { retryAfterSecs: retryAfter }
      );
    }

    if (!buildRes.ok) {
      const detail = await buildRes.text();
      throw new X402Error(
        'BUILD_FAILED',
        `Facilitator build-exact-payment-tx returned HTTP ${buildRes.status}: ${detail}`,
        { httpStatus: buildRes.status }
      );
    }

    const buildJson = await buildRes.json();

    // BUY-3: Check blockhash expiry before signing
    if (buildJson.recentBlockhashExpiresAt) {
      const nowSec = Math.floor(Date.now() / 1000);
      if (nowSec >= buildJson.recentBlockhashExpiresAt) {
        throw new X402Error(
          'BLOCKHASH_EXPIRED',
          `The embedded blockhash expired at UNIX ${buildJson.recentBlockhashExpiresAt}. Request a fresh build from the Facilitator.`,
          { expiresAt: buildJson.recentBlockhashExpiresAt }
        );
      }
    }

    if (!buildJson.verifyBodyTemplate)
      throw new X402Error(
        'MISSING_VERIFY_TEMPLATE',
        "Facilitator response is missing 'verifyBodyTemplate'. The Facilitator may be running an incompatible version."
      );

    if (!buildJson.transaction)
      throw new X402Error(
        'MISSING_TRANSACTION',
        "Facilitator response is missing 'transaction'. The Facilitator may be running an incompatible version."
      );

    // ── Step 3: Sign the unsigned transaction ───────────────────────
    const txBytes = Uint8Array.from(
      atob(buildJson.transaction),
      (c) => c.charCodeAt(0)
    );
    const vtx = VersionedTransaction.deserialize(txBytes);
    vtx.sign([this.wallet]);

    const signedB64 = btoa(
      String.fromCharCode(...vtx.serialize())
    );

    // ── Step 4: Inject signature into verify body template ──────────
    const verifyBody = buildJson.verifyBodyTemplate;
    verifyBody.paymentPayload.payload.transaction = signedB64;

    const proofB64 = btoa(JSON.stringify(verifyBody));

    // ── Step 5: Replay original request with proof ──────────────────
    return fetch(url, {
      ...options,
      headers: {
        ...(options?.headers || {}),
        'X-PAYMENT': proofB64,
      },
    });
  }
}
