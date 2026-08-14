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
  | 'UNTRUSTED_FACILITATOR'
  | 'PAYMENT_LIMIT_EXCEEDED'
  | 'INCONSISTENT_BUILD'
  | 'INVALID_TRANSACTION'
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

export interface X402AgentClientOptions {
  /** Facilitator origins this wallet may trust with transaction construction. */
  trustedFacilitatorOrigins?: readonly string[];
  /** Optional maximum accepted.amount, in the mint's atomic units. */
  maxPaymentAmount?: string;
}

export const DEFAULT_TRUSTED_FACILITATOR_ORIGINS = [
  'https://ipay.sh',
  'https://agent.pay402.me',
  'https://preview.ipay.sh',
  'https://preview.agent.pay402.me',
] as const;

type PaymentRule = Record<string, unknown> & {
  scheme?: string;
  network?: string;
  asset?: string;
  amount?: string;
  payTo?: string;
  extra?: Record<string, unknown>;
};

function isExactSolanaRule(rule: PaymentRule): boolean {
  return (
    (rule.scheme === 'exact' || rule.scheme === 'v2:solana:exact') &&
    typeof rule.network === 'string' &&
    rule.network.startsWith('solana:')
  );
}

function normalizedExactScheme(value: unknown): unknown {
  return value === 'v2:solana:exact' ? 'exact' : value;
}

function parseAtomicAmount(value: unknown, label: string): bigint {
  if (typeof value !== 'string' || !/^\d+$/.test(value)) {
    throw new X402Error(
      'INCONSISTENT_BUILD',
      `${label} must be a non-negative integer string.`
    );
  }
  return BigInt(value);
}

function facilitatorBaseFromCapabilitiesUrl(capabilitiesUrl: string): URL {
  let parsed: URL;
  try {
    parsed = new URL(capabilitiesUrl);
  } catch {
    throw new X402Error(
      'UNTRUSTED_FACILITATOR',
      `Invalid facilitator capabilities URL: ${capabilitiesUrl}`
    );
  }
  if (!parsed.pathname.endsWith('/capabilities')) {
    throw new X402Error(
      'UNTRUSTED_FACILITATOR',
      `Facilitator capabilities URL must end with /capabilities: ${capabilitiesUrl}`
    );
  }
  parsed.pathname = parsed.pathname.slice(0, -'/capabilities'.length);
  parsed.search = '';
  parsed.hash = '';
  return parsed;
}

function assertBuildMatchesRule(buildJson: Record<string, unknown>, rule: PaymentRule): void {
  const template = buildJson.verifyBodyTemplate;
  if (!template || typeof template !== 'object') {
    throw new X402Error(
      'MISSING_VERIFY_TEMPLATE',
      "Facilitator response is missing 'verifyBodyTemplate'."
    );
  }
  const requirements = (template as Record<string, unknown>).paymentRequirements;
  if (!requirements || typeof requirements !== 'object') {
    throw new X402Error(
      'MISSING_VERIFY_TEMPLATE',
      "Facilitator verifyBodyTemplate is missing 'paymentRequirements'."
    );
  }

  const built = requirements as PaymentRule;
  for (const field of ['scheme', 'network', 'asset', 'amount', 'payTo'] as const) {
    const expected = field === 'scheme' ? normalizedExactScheme(rule[field]) : rule[field];
    const actual = field === 'scheme' ? normalizedExactScheme(built[field]) : built[field];
    if (expected !== actual) {
      throw new X402Error(
        'INCONSISTENT_BUILD',
        `Facilitator changed payment term '${field}' in verifyBodyTemplate.`
      );
    }
  }
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
  private trustedFacilitatorOrigins: ReadonlySet<string>;
  private maxPaymentAmount?: bigint;

  constructor(wallet: Keypair, options: X402AgentClientOptions = {}) {
    this.wallet = wallet;
    const origins =
      options.trustedFacilitatorOrigins ?? DEFAULT_TRUSTED_FACILITATOR_ORIGINS;
    this.trustedFacilitatorOrigins = new Set(origins.map((origin) => new URL(origin).origin));
    if (options.maxPaymentAmount !== undefined) {
      this.maxPaymentAmount = parseAtomicAmount(
        options.maxPaymentAmount,
        'maxPaymentAmount'
      );
    }
  }

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
  async fetchWithAutoPay(
    url: string,
    preferredMint: string,
    options?: FetchAutoPayOptions
  ): Promise<Response> {
    const { autoWrapSol, ...requestOptions } = options ?? {};
    const res = await fetch(url, requestOptions);

    if (res.status === 200) return res;
    if (res.status !== 402)
      throw new X402Error(
        'UNEXPECTED_STATUS',
        `Unexpected HTTP status ${res.status}. Expected 200 (free) or 402 (payment required).`,
        { httpStatus: res.status }
      );

    // ── Step 1: Parse the 402 Challenge ─────────────────────────────
    const requirement = await res.json();
    const accepts: PaymentRule[] = requirement.accepts || [];

    if (accepts.length === 0)
      throw new X402Error(
        'MISSING_ACCEPTS',
        "The 402 response has no 'accepts' array. The Resource Provider's payment configuration is invalid. Contact the RP operator."
      );

    const availableMints = accepts
      .filter(isExactSolanaRule)
      .map((a) => a.asset)
      .filter((asset): asset is string => typeof asset === 'string');

    const rule = accepts.find(
      (candidate) => isExactSolanaRule(candidate) && candidate.asset === preferredMint
    );
    if (!rule)
      throw new X402Error(
        'MINT_NOT_ACCEPTED',
        `Resource does not accept mint ${preferredMint}. Available mints: [${availableMints.join(', ')}]. Pick one from this list.`,
        { availableMints }
      );

    const paymentAmount = parseAtomicAmount(rule.amount, 'accepted.amount');
    if (this.maxPaymentAmount !== undefined && paymentAmount > this.maxPaymentAmount) {
      throw new X402Error(
        'PAYMENT_LIMIT_EXCEEDED',
        `Payment amount ${paymentAmount} exceeds configured maximum ${this.maxPaymentAmount}.`
      );
    }

    const capUrlValue = rule.extra?.capabilitiesUrl;
    const capUrl = typeof capUrlValue === 'string' ? capUrlValue : undefined;
    if (!capUrl)
      throw new X402Error(
        'MISSING_CAPABILITIES_URL',
        'This 402-gated resource did not provide extra.capabilitiesUrl. The Resource Provider has not completed Facilitator integration. See public/onboarding_guide.md (GET /onboarding_guide.md on the facilitator host).'
      );

    // ── Step 2: Ask Facilitator to build the tx ─────────────────────
    const facilitatorBase = facilitatorBaseFromCapabilitiesUrl(capUrl);
    if (!this.trustedFacilitatorOrigins.has(facilitatorBase.origin)) {
      throw new X402Error(
        'UNTRUSTED_FACILITATOR',
        `Refusing to send a signing request to untrusted facilitator origin ${facilitatorBase.origin}. Add it explicitly to trustedFacilitatorOrigins if intended.`
      );
    }
    const buildRes = await fetch(
      `${facilitatorBase.toString().replace(/\/$/, '')}/build-exact-payment-tx`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          payer: this.wallet.publicKey.toBase58(),
          accepted: rule,
          resource: requirement.resource,
          skipSourceBalanceCheck: true,
          autoWrapSol,
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

    const buildJson = (await buildRes.json()) as Record<string, unknown>;

    // BUY-3: Check blockhash expiry before signing
    if (typeof buildJson.recentBlockhashExpiresAt === 'number') {
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

    assertBuildMatchesRule(buildJson, rule);

    // ── Step 3: Sign the unsigned transaction ───────────────────────
    const txBytes = Uint8Array.from(
      atob(buildJson.transaction as string),
      (c) => c.charCodeAt(0)
    );
    const vtx = VersionedTransaction.deserialize(txBytes);
    const payerSignatureIndex = buildJson.payerSignatureIndex;
    if (
      typeof payerSignatureIndex !== 'number' ||
      !Number.isSafeInteger(payerSignatureIndex) ||
      payerSignatureIndex < 0 ||
      payerSignatureIndex >= vtx.message.header.numRequiredSignatures ||
      vtx.message.staticAccountKeys[payerSignatureIndex]?.toBase58() !==
        this.wallet.publicKey.toBase58()
    ) {
      throw new X402Error(
        'INVALID_TRANSACTION',
        'Facilitator transaction does not assign the declared payer signature slot to this wallet.'
      );
    }
    vtx.sign([this.wallet]);

    const signedB64 = btoa(
      String.fromCharCode(...vtx.serialize())
    );

    // ── Step 4: Inject signature into verify body template ──────────
    const verifyBody = buildJson.verifyBodyTemplate as {
      paymentPayload?: { payload?: { transaction?: string } };
    };
    if (!verifyBody.paymentPayload?.payload) {
      throw new X402Error(
        'MISSING_VERIFY_TEMPLATE',
        "Facilitator verifyBodyTemplate is missing 'paymentPayload.payload'."
      );
    }
    verifyBody.paymentPayload.payload.transaction = signedB64;

    const proofB64 = btoa(JSON.stringify(verifyBody));

    // ── Step 5: Replay original request with proof ──────────────────
    //
    // x402 v2 uses the `PAYMENT-SIGNATURE` header name (see the x402 HTTP
    // transport-v2 spec and `public/agent-integration.md` in this repo).
    // v1 used `X-PAYMENT`; every seller in this ecosystem today — aethervane,
    // spl-token-balance-serverless, x402-seller-starter — reads only
    // `PAYMENT-SIGNATURE`, so emitting `X-PAYMENT` silently fails with a
    // repeated 402. Emit the canonical v2 header exclusively.
    const retryHeaders = new Headers(requestOptions.headers);
    retryHeaders.set('PAYMENT-SIGNATURE', proofB64);
    return fetch(url, { ...requestOptions, headers: retryHeaders });
  }
}
