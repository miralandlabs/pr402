import { Keypair, VersionedTransaction } from '@solana/web3.js';

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
   * @param options    - Optional extra fetch options (headers, etc.)
   */
  async fetchWithAutoPay(
    url: string,
    preferredMint: string,
    options?: RequestInit
  ): Promise<Response> {
    const res = await fetch(url, options);

    if (res.status === 200) return res;
    if (res.status !== 402)
      throw new Error(`Unexpected HTTP status: ${res.status}`);

    // ── Step 1: Parse the 402 Challenge ─────────────────────────────
    const requirement = await res.json();
    const accepts: any[] = requirement.accepts || [];

    const rule = accepts.find((a: any) => a.asset === preferredMint);
    if (!rule)
      throw new Error(
        `Resource does not accept preferred mint: ${preferredMint}`
      );

    const capUrl: string | undefined = rule.extra?.capabilitiesUrl;
    if (!capUrl)
      throw new Error('Missing capabilitiesUrl in 402 extra block');

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
        }),
      }
    );

    if (!buildRes.ok)
      throw new Error(`Facilitator build failed: ${await buildRes.text()}`);

    const buildJson = await buildRes.json();

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
