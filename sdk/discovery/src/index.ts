export type SearchResourcesOptions = {
  q?: string;
  category?: string;
  scheme?: 'exact' | 'sla-escrow';
  tag?: string;
  limit?: number;
  cursor?: string;
};

export type PublicResourceEntry = {
  id: number;
  walletPubkey: string;
  resourceUrl: string;
  httpMethod: string;
  title: string;
  description?: string;
  useCase?: string;
  category?: string;
  tags: string[];
  scheme: string;
  network?: string;
  intentContractUrl?: string;
  merchantOrigin?: string;
  registrationVerifiedAt: string;
  updatedAt: string;
};

export type SearchResourcesResponse = {
  entries: PublicResourceEntry[];
  nextCursor?: string;
  notice?: string;
};

export type ProbeSummary = {
  ok: boolean;
  httpStatus?: number;
  scheme?: string;
  error?: string;
  acceptsSummary?: { scheme?: string; amount?: string; asset?: string };
};

function trimBase(url: string): string {
  return url.replace(/\/$/, '');
}

export async function searchResources(
  facilitatorUrl: string,
  opts: SearchResourcesOptions = {}
): Promise<SearchResourcesResponse> {
  const base = trimBase(facilitatorUrl);
  const params = new URLSearchParams();
  if (opts.q) params.set('q', opts.q);
  if (opts.category) params.set('category', opts.category);
  if (opts.scheme) params.set('scheme', opts.scheme);
  if (opts.tag) params.set('tag', opts.tag);
  if (opts.limit) params.set('limit', String(opts.limit));
  if (opts.cursor) params.set('cursor', opts.cursor);
  const qs = params.toString();
  const res = await fetch(
    `${base}/api/v1/facilitator/resources${qs ? `?${qs}` : ''}`
  );
  if (!res.ok) {
    throw new Error(`searchResources HTTP ${res.status}: ${await res.text()}`);
  }
  return (await res.json()) as SearchResourcesResponse;
}

export async function getResource(
  facilitatorUrl: string,
  resourceUrl: string
): Promise<PublicResourceEntry | undefined> {
  const q = encodeURIComponent(resourceUrl);
  const data = await searchResources(facilitatorUrl, { q: resourceUrl, limit: 20 });
  return data.entries.find((e) => e.resourceUrl === resourceUrl);
}

export async function probeResource(
  resourceUrl: string,
  httpMethod = 'GET'
): Promise<ProbeSummary> {
  const res = await fetch(resourceUrl, { method: httpMethod, redirect: 'manual' });
  if (res.status !== 402) {
    return { ok: false, httpStatus: res.status, error: `expected 402, got ${res.status}` };
  }
  let body: Record<string, unknown>;
  try {
    body = (await res.json()) as Record<string, unknown>;
  } catch (e) {
    return { ok: false, httpStatus: 402, error: `402 body not JSON: ${String(e)}` };
  }
  const accepts = body.accepts as Array<Record<string, unknown>> | undefined;
  const line = accepts?.[0];
  const resource = body.resource as Record<string, unknown> | undefined;
  if (resource?.url !== resourceUrl) {
    return {
      ok: false,
      httpStatus: 402,
      error: `resource.url mismatch (${String(resource?.url)})`,
    };
  }
  if (!line?.scheme) {
    return { ok: false, httpStatus: 402, error: 'missing accepts[0].scheme' };
  }
  return {
    ok: true,
    httpStatus: 402,
    scheme: String(line.scheme),
    acceptsSummary: {
      scheme: String(line.scheme),
      amount: line.amount != null ? String(line.amount) : undefined,
      asset: line.asset != null ? String(line.asset) : undefined,
    },
  };
}
