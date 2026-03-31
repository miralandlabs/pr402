/**
 * Thin HTTP helpers for pr402 facilitator APIs (discovery, build, verify, settle).
 *
 * Paths match [public/openapi.json](../public/openapi.json). Zero heavy dependencies (`fetch` only).
 *
 * **Rust:** same contract in `pr402::sdk::http` behind Cargo feature `facilitator-http`
 * (`FacilitatorHttpClient` + free async functions).
 */

export const BUILD_EXACT_PAYMENT_TX_PATH =
  "/api/v1/facilitator/build-exact-payment-tx";

export const BUILD_SLA_ESCROW_PAYMENT_TX_PATH =
  "/api/v1/facilitator/build-sla-escrow-payment-tx";

export const FACILITATOR_SUPPORTED_PATH = "/api/v1/facilitator/supported";
export const FACILITATOR_HEALTH_PATH = "/api/v1/facilitator/health";
export const FACILITATOR_CAPABILITIES_PATH = "/api/v1/facilitator/capabilities";
export const FACILITATOR_VERIFY_PATH = "/api/v1/facilitator/verify";
export const FACILITATOR_SETTLE_PATH = "/api/v1/facilitator/settle";
/** Static OpenAPI 3.1 document (same origin as facilitator). */
export const FACILITATOR_OPENAPI_PATH = "/openapi.json";
/** Markdown agent runbook (source `src/agent_integration.md`, embedded in facilitator). */
export const FACILITATOR_AGENT_INTEGRATION_PATH = "/agent-integration.md";

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

/** x402 v2 verify/settle POST body (superset; see OpenAPI `X402V2VerifySettleBody`). */
export type X402V2VerifySettleBody = {
  x402Version: 2;
  paymentPayload: unknown;
  paymentRequirements: unknown;
  correlationId?: string;
  [key: string]: unknown;
};

function root(baseUrl: string): string {
  return baseUrl.replace(/\/$/, "");
}

async function getJson<T>(baseUrl: string, path: string): Promise<T> {
  const url = `${root(baseUrl)}${path}`;
  const res = await fetch(url, { method: "GET" });
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`${path} HTTP ${res.status}: ${text}`);
  }
  return JSON.parse(text) as T;
}

async function postJson<T>(
  baseUrl: string,
  path: string,
  body: unknown,
  headers?: Record<string, string>,
): Promise<T> {
  const url = `${root(baseUrl)}${path}`;
  const res = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...headers,
    },
    body: JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`${path} HTTP ${res.status}: ${text}`);
  }
  return JSON.parse(text) as T;
}

/** Fetch OpenAPI 3.1 JSON (`/openapi.json`). */
export function fetchFacilitatorOpenApi(
  facilitatorBaseUrl: string,
): Promise<unknown> {
  return getJson(facilitatorBaseUrl, FACILITATOR_OPENAPI_PATH);
}

/** `GET .../supported` */
export function getSupported(facilitatorBaseUrl: string): Promise<unknown> {
  return getJson(facilitatorBaseUrl, FACILITATOR_SUPPORTED_PATH);
}

/** `GET .../health` (same body semantics as supported). */
export function getHealth(facilitatorBaseUrl: string): Promise<unknown> {
  return getJson(facilitatorBaseUrl, FACILITATOR_HEALTH_PATH);
}

/** `GET .../capabilities` — discovery blob including `httpEndpoints.openApi`. */
export function getCapabilities(facilitatorBaseUrl: string): Promise<unknown> {
  return getJson(facilitatorBaseUrl, FACILITATOR_CAPABILITIES_PATH);
}

/** `POST .../verify` — optional `X-Correlation-Id` header. */
export function verifyPayment(
  facilitatorBaseUrl: string,
  body: X402V2VerifySettleBody,
  correlationId?: string,
): Promise<unknown> {
  const headers: Record<string, string> = {};
  if (correlationId) headers["X-Correlation-Id"] = correlationId;
  return postJson(facilitatorBaseUrl, FACILITATOR_VERIFY_PATH, body, headers);
}

/** `POST .../settle` — reuse same body and correlation id as verify. */
export function settlePayment(
  facilitatorBaseUrl: string,
  body: X402V2VerifySettleBody,
  correlationId?: string,
): Promise<unknown> {
  const headers: Record<string, string> = {};
  if (correlationId) headers["X-Correlation-Id"] = correlationId;
  return postJson(facilitatorBaseUrl, FACILITATOR_SETTLE_PATH, body, headers);
}

/** `POST .../build-exact-payment-tx` */
export function buildExactPaymentTx(
  facilitatorBaseUrl: string,
  body: BuildExactPaymentTxRequest,
): Promise<BuildPaymentTxResponse> {
  return postJson(facilitatorBaseUrl, BUILD_EXACT_PAYMENT_TX_PATH, body);
}

/** `POST .../build-sla-escrow-payment-tx` */
export function buildSlaEscrowPaymentTx(
  facilitatorBaseUrl: string,
  body: BuildSlaEscrowPaymentTxRequest,
): Promise<BuildPaymentTxResponse> {
  return postJson(facilitatorBaseUrl, BUILD_SLA_ESCROW_PAYMENT_TX_PATH, body);
}
