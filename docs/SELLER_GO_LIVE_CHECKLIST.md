# Seller go-live checklist (support)

Internal mirror of the **seller-facing 6-step path** on ipay.sh. Use the same steps on support calls — do not introduce a different process.

**Seller UI (canonical):** [ipay.sh/#seller-lifecycle](https://ipay.sh/#seller-lifecycle) → steps 2–4 · [/resources](https://ipay.sh/resources) → steps 5–6

**Devnet rehearsal:** [preview.ipay.sh](https://preview.ipay.sh) — same steps, same copy.

---

## The 6 steps (seller language)

| Step | Seller sees | Done when |
|------|-------------|-----------|
| 1 | API returns **402** when unpaid | Seller’s endpoint returns 402 + valid PaymentRequired JSON on unpaid request |
| 2 | **Activate** payment vault | Wallet signed provision-tx; vault on-chain |
| 3 | **Register shop** — API website + sign | `registration_verified_at` set; `discovery.serviceUrl` host recorded |
| 4 | Preview vault *(optional)* | Seller skipped or ran preview — not required for directory |
| 5 | **Add your API** | `POST /resources/register` succeeded for `resourceUrl` |
| 6 | **We verify 402** | `last_probe_ok: true` on resource row |

**Success:** row visible on `GET /resources` and home `#directory`.

---

## Support walkthrough

### Step 1 — 402 on seller server

- Seller integrates per [quickstart-seller](https://docs.ipay.sh/quickstart-seller.html).
- Unpaid `curl` to payable path must return **HTTP 402** (not 400/200).
- If 400: add query params / fix path so request reaches payment gate.

### Step 2 — Activate (home page)

1. Connect wallet on [go live · seller](https://ipay.sh/#seller-lifecycle).
2. Click **activate** (USDC default).
3. Confirm transaction in wallet.

**Blockers:** insufficient SOL for fees; wrong cluster (preview vs mainnet).

### Step 3 — Register shop (home page)

1. Enter **API website** (origin URL — required for directory).
2. Connect wallet → **sign & register shop**.
3. Must complete step 2 first (409 if vault missing).

**Blockers:** empty API website; pubkey field ≠ connected wallet.

### Step 4 — Preview (optional)

Skip unless seller wants vault PDAs before activate.

### Step 5 — Add your API ([/resources](https://ipay.sh/resources))

1. Connect same wallet.
2. Pre-flight banner should be hidden if steps 2–3 done.
3. Enter payable **resource URL** (same host as step 3 API website).
4. **Show in public directory** checked (default).
5. **Sign & add API**.

**Blockers:** 403 — step 3 not done; host mismatch between API website and resource URL.

### Step 6 — Auto 402 verify

Runs automatically after step 5 when listing publicly. UI shows plain-English pass/fail.

**Blockers:** probe not 402; `resource.url` path mismatch; endpoint down.

---

## Curl appendix (support only — not seller-facing)

Replace `$BASE` with `https://ipay.sh` or `https://preview.ipay.sh`.

```bash
# Cluster / features
curl -sS "$BASE/api/v1/facilitator/health" | jq .
curl -sS "$BASE/api/v1/facilitator/capabilities" | jq '.features.publicResourceDirectory'

# Seller lifecycle (paste seller pubkey)
WALLET='<seller_base58>'
curl -sS "$BASE/api/v1/facilitator/sellers/$WALLET/preview" | jq '.lifecycle'

# Public directory
curl -sS "$BASE/api/v1/facilitator/resources?limit=5" | jq '.entries[] | {title, resourceUrl, walletPubkey}'

# Probe unpaid endpoint (seller-side sanity)
curl -sS -o /dev/null -w '%{http_code}\n' 'https://seller-api.example/path'
# expect: 402
```

### Gate mapping (developer reference)

| Public listing gate | Backend |
|-------------------|---------|
| Verified merchant | Layer 2 · `registration_verified_at` |
| `serviceUrl` host | Layer 2 · `discovery.serviceUrl` |
| Resource row | Layer 3 · `payable_resources` |
| `listingOptIn: true` | Layer 3 register body |
| `last_probe_ok: true` | Probe or indexer harvest |

See [DISCOVERY.md](./DISCOVERY.md) for agent integrators.

---

## Validation log

| Environment | Date | API smoke | Seller UI journey |
|-------------|------|-----------|-------------------|
| preview (devnet) | 2026-06-03 | health OK · devnet · `publicResourceDirectory: true` | Checklist + steps 5–6 UI shipped in repo; redeploy pr402 to preview |
| production (mainnet) | 2026-06-03 | health OK · mainnet · 2+ public resources | Same — redeploy pr402 to ipay.sh |

**Post-deploy UI checks (seller-only path):**

1. Home shows **Go live in 6 steps** checklist with no Layer 2/3 jargon.
2. Step 3 shows **API website** field (not hidden in details).
3. `/resources` shows **Step 5 of 6** + pre-flight when steps 2–3 incomplete.
4. Register with “Show in public directory” runs step 6 automatically.
5. Pass → row on `GET /resources` and `#directory`.

Fill the “Seller UI journey” column after one full wallet walkthrough per environment.
