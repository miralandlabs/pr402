"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.searchResources = searchResources;
exports.probeResource = probeResource;
async function searchResources(facilitatorUrl, options = {}) {
    const params = new URLSearchParams();
    if (options.q)
        params.set('q', options.q);
    if (options.category)
        params.set('category', options.category);
    if (options.scheme)
        params.set('scheme', options.scheme);
    if (options.tag)
        params.set('tag', options.tag);
    if (options.limit)
        params.set('limit', String(options.limit));
    const query = params.toString();
    const base = facilitatorUrl.replace(/\/$/, '');
    const response = await fetch(`${base}/resources${query ? `?${query}` : ''}`);
    if (!response.ok) {
        throw new Error(`searchResources HTTP ${response.status}: ${await response.text()}`);
    }
    return response.json();
}
async function probeResource(resourceUrl, httpMethod = 'GET') {
    const response = await fetch(resourceUrl, { method: httpMethod, redirect: 'manual' });
    if (response.status !== 402) {
        return {
            ok: false,
            httpStatus: response.status,
            error: `expected 402, got ${response.status}`,
        };
    }
    let body;
    try {
        body = (await response.json());
    }
    catch (error) {
        return { ok: false, httpStatus: 402, error: `402 body not JSON: ${String(error)}` };
    }
    const accepts = body.accepts;
    const line = accepts?.[0];
    const resource = body.resource;
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
            amount: line.amount == null ? undefined : String(line.amount),
            asset: line.asset == null ? undefined : String(line.asset),
        },
    };
}
