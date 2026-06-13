---
title: "Hands-on seller lab"
---

# Hands-on seller lab

**Goal:** Complete all **6 go-live steps** on [preview.ipay.sh](https://preview.ipay.sh/#seller-lifecycle).

This lab uses **preview.ipay.sh + Solana Devnet + Devnet USDC**. Production later uses **ipay.sh + Solana Mainnet + real USDC**.

**Start here:** [x402-seller-lab-express](https://github.com/miralandlabs/x402/tree/main/x402-seller-lab-express) — not [x402-seller-starter](https://github.com/miralandlabs/x402-seller-starter) (Rust; same idea, different repo).

Full steps: repo **README.md**.

---

## Lab ↔ preview.ipay.sh

| preview.ipay.sh | README section |
|---------|----------------|
| **1** — API returns 402 | **A → B → C** |
| **2** — Preview vault | Skip |
| **3** — **Activate** (self-provision) | **D** |
| **4** — Register shop | **D** |
| **5** — Add API | **D** |
| **6** — Verify 402 | automatic |

**Done** when step 6 passes and you appear in [#directory](https://preview.ipay.sh/#directory).

### Why Activate (step 3)?

One wallet signature (~0.1 SOL) creates your on-chain payment vault **before** the first sale.

- **You:** pr402 Facilitator charges **90 bps** protocol fee (sovereign) vs **100 bps** if you skip and wait for JIT provision on first payment  
- **pr402:** no vault-creation cost bundled into a buyer's first settle  

Self-provision is the win-win path pr402 is designed around.

---

## Quick path

**A** — Phantom or Solflare browser extension → Devnet + SOL ([faucet](https://faucet.solana.com/))  

**B** — `npm install` → `.env` → `enrich` + `dev` → `verify-402` localhost → **PASS**  

**C** — deploy to public **https** (any host; Vercel example in repo) → `enrich` again → `verify-402` live → **PASS**  

**D** — [preview.ipay.sh](https://preview.ipay.sh/#seller-lifecycle): **Activate** → Register → [resources](https://preview.ipay.sh/resources) → wait for step 6  

**E** — copy 3 files into your real project (repo README)

[Integrate your API](/seller-quick-start.html) · [Fees](/start-here.html#appendix-a-protocol-fees--pricing)
