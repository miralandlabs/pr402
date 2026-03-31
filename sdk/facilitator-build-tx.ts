/**
 * Optional thin HTTP helpers for pr402 facilitator **payment transaction build** endpoints.
 *
 * Paths are explicit so agents never confuse SLA-Escrow fund-payment builds with `exact` builds.
 */

export const BUILD_EXACT_PAYMENT_TX_PATH =
  "/api/v1/facilitator/build-exact-payment-tx";

export const BUILD_SLA_ESCROW_PAYMENT_TX_PATH =
  "/api/v1/facilitator/build-sla-escrow-payment-tx";

export type BuildExactPaymentTxRequest = {
  payer: string;
  accepted: unknown;
  resource: unknown;
  skipSourceBalanceCheck?: boolean;
};

export type BuildSlaEscrowPaymentTxRequest = {
  payer: string;
  accepted: unknown;
  resource: unknown;
  slaHash: string;
  oracleAuthority: string;
  paymentUid?: string;
  skipSourceBalanceCheck?: boolean;
  /** `true` = buyer fee payer (CLI-shaped). Omit/false = facilitator pays fees (`exact`-aligned). */
  buyerPaysTransactionFees?: boolean;
};

export type BuildPaymentTxResponse = {
  x402Version: number;
  transaction: string;
  recentBlockhash: string;
  feePayer: string;
  payer: string;
  verifyBodyTemplate?: unknown;
  paymentUid?: string;
  notes?: string[];
};

async function postJson<T>(
  baseUrl: string,
  path: string,
  body: unknown,
): Promise<T> {
  const url = `${baseUrl.replace(/\/$/, "")}${path}`;
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`${path} HTTP ${res.status}: ${text}`);
  }
  return JSON.parse(text) as T;
}

/** `POST .../build-exact-payment-tx` */
export function buildExactPaymentTx(
  facilitatorBaseUrl: string,
  body: BuildExactPaymentTxRequest,
): Promise<BuildPaymentTxResponse> {
  return postJson(facilitatorBaseUrl, BUILD_EXACT_PAYMENT_TX_PATH, body);
}

/** `POST .../build-sla-escrow-payment-tx` (FundPayment shell; buyer fee payer). */
export function buildSlaEscrowPaymentTx(
  facilitatorBaseUrl: string,
  body: BuildSlaEscrowPaymentTxRequest,
): Promise<BuildPaymentTxResponse> {
  return postJson(facilitatorBaseUrl, BUILD_SLA_ESCROW_PAYMENT_TX_PATH, body);
}
