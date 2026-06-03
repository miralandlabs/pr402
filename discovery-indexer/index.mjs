#!/usr/bin/env node
/**
 * Discovery indexer: harvest SRM manifests, probe 402, publish resource-index.json.
 *
 * Usage:
 *   FACILITATOR_URL=https://preview.ipay.sh node index.mjs
 *   DATABASE_URL=postgres://... FACILITATOR_URL=... node index.mjs --harvest
 *
 * Without DATABASE_URL, only rebuilds index from GET /resources (no harvest upsert).
 */

import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, "..");
const OUT = join(ROOT, "public/dist/resource-index.json");

const facilitator = (process.env.FACILITATOR_URL || "https://preview.ipay.sh").replace(
  /\/$/,
  "",
);
const dbUrl = process.env.DATABASE_URL;

async function fetchJson(url) {
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`${url} → ${res.status}`);
  return res.json();
}

// Compare two URLs on scheme + host + path only, ignoring the query string: the
// probed URL may carry example params (to reach the 402 gate instead of a 400),
// while the seller's 402 may advertise a canonical resource.url without them.
function sameOriginPath(a, b) {
  try {
    const ua = new URL(a);
    const ub = new URL(b);
    return ua.protocol === ub.protocol && ua.host === ub.host && ua.pathname === ub.pathname;
  } catch {
    return false;
  }
}

async function probeResource(resourceUrl, httpMethod = "GET") {
  const method = httpMethod.toUpperCase();
  const res = await fetch(resourceUrl, { method, redirect: "manual" });
  if (res.status !== 402) {
    return { ok: false, httpStatus: res.status, error: `expected 402, got ${res.status}` };
  }
  let body;
  try {
    body = await res.json();
  } catch (e) {
    return { ok: false, httpStatus: 402, error: `402 body not JSON: ${e}` };
  }
  const scheme = body?.accepts?.[0]?.scheme;
  const url = body?.resource?.url;
  if (!scheme) return { ok: false, httpStatus: 402, error: "missing accepts[0].scheme" };
  if (!sameOriginPath(url, resourceUrl)) {
    return { ok: false, httpStatus: 402, error: `resource.url origin/path mismatch (${url})` };
  }
  return { ok: true, httpStatus: 402, scheme };
}

async function harvestManifests() {
  if (!dbUrl) {
    console.warn("DATABASE_URL unset — skipping manifest harvest upsert");
    return;
  }
  const { default: pg } = await import("pg");
  const pool = new pg.Pool({ connectionString: dbUrl });
  try {
    const { rows: providers } = await pool.query(`
      SELECT wallet_pubkey, service_url
        FROM resource_providers
       WHERE listing_opt_in = TRUE
         AND registration_verified_at IS NOT NULL
         AND inactive = FALSE
         AND retired_at IS NULL
         AND service_url IS NOT NULL
         AND service_url <> ''
    `);

    for (const p of providers) {
      const origin = p.service_url.replace(/\/$/, "");
      const manifestUrl = `${origin}/.well-known/x402-resources.json`;
      let manifest;
      try {
        manifest = await fetchJson(manifestUrl);
      } catch (e) {
        console.warn(`skip ${manifestUrl}: ${e.message}`);
        continue;
      }
      if (!Array.isArray(manifest.resources)) continue;

      for (const r of manifest.resources) {
        if (!r.resourceUrl || !r.title || !r.scheme) continue;
        if (!r.id) {
          console.warn(`skip ${r.resourceUrl}: manifest resource needs an "id" (harvest dedupes on wallet+id)`);
          continue;
        }
        const hostOk =
          new URL(r.resourceUrl).host.toLowerCase() === new URL(origin).host.toLowerCase();
        if (!hostOk) {
          console.warn(`origin mismatch ${r.resourceUrl} vs ${origin}`);
          continue;
        }
        await pool.query(
          `INSERT INTO payable_resources (
            wallet_pubkey, resource_provider_id, resource_url, http_method, seller_resource_id,
            title, description, use_case, category, tags, scheme,
            intent_contract_url, listing_opt_in, registration_verified_at,
            source, manifest_origin, updated_at
          )
          SELECT $1, rp.id, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, TRUE, NOW(), 'manifest_harvest', $12, NOW()
          FROM resource_providers rp
          WHERE rp.wallet_pubkey = $1 AND rp.registration_verified_at IS NOT NULL AND rp.retired_at IS NULL
          LIMIT 1
          ON CONFLICT (wallet_pubkey, seller_resource_id)
            WHERE seller_resource_id IS NOT NULL AND retired_at IS NULL
          DO UPDATE SET
            resource_url = EXCLUDED.resource_url,
            http_method = EXCLUDED.http_method,
            title = EXCLUDED.title,
            description = EXCLUDED.description,
            use_case = EXCLUDED.use_case,
            category = EXCLUDED.category,
            tags = EXCLUDED.tags,
            scheme = EXCLUDED.scheme,
            intent_contract_url = EXCLUDED.intent_contract_url,
            source = 'manifest_harvest',
            manifest_origin = EXCLUDED.manifest_origin,
            updated_at = NOW()`,
          [
            p.wallet_pubkey,
            r.resourceUrl,
            (r.method || "GET").toUpperCase(),
            r.id || null,
            r.title,
            r.description || null,
            r.useCase || null,
            r.category || null,
            r.tags || null,
            r.scheme,
            r.intentContractUrl || null,
            origin,
          ],
        );

        const probe = await probeResource(r.resourceUrl, r.method || "GET");
        await pool.query(
          `UPDATE payable_resources SET
            last_probe_at = NOW(),
            last_probe_ok = $2,
            last_probe_http_status = $3,
            last_probe_scheme = $4,
            last_probe_error = $5,
            updated_at = NOW()
           WHERE resource_url = $1`,
          [
            r.resourceUrl,
            probe.ok,
            probe.httpStatus ?? null,
            probe.scheme ?? null,
            probe.error ?? null,
          ],
        );
      }
    }
  } finally {
    await pool.end();
  }
}

async function buildIndex() {
  const data = await fetchJson(`${facilitator}/api/v1/facilitator/resources?limit=100`);
  const index = {
    schemaVersion: "1.0.0",
    generatedAt: new Date().toISOString(),
    facilitator,
    notice: data.notice,
    entries: data.entries || [],
  };
  mkdirSync(dirname(OUT), { recursive: true });
  writeFileSync(OUT, JSON.stringify(index, null, 2) + "\n");
  console.log(`Wrote ${OUT} (${index.entries.length} entries)`);
}

const harvest = process.argv.includes("--harvest");
(async () => {
  if (harvest) await harvestManifests();
  await buildIndex();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
