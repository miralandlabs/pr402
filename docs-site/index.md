---
layout: home

hero:
  name: "pr402"
  text: "x402 facilitator for Solana"
  tagline: "HTTP 402 Payment Required for machines. Settled on-chain via UniversalSettle (exact) and SLA-Escrow(sla-escrow). Live on Mainnet and Devnet."
  image:
    src: /pr402.png
    alt: pr402
  actions:
    - theme: brand
      text: Seller Quickstart
      link: /seller-quick-start
    - theme: alt
      text: Buyer Quickstart
      link: /quickstart-buyer
    - theme: alt
      text: Install buyer CLI
      link: /quickstart-buyer#install
    - theme: alt
      text: Agent integration runbook
      link: /agent-integration

features:
  - title: Sellers / Resource Providers
    details: "UniversalSettle split-vault onboarding, sovereign discount, optional signed registry, SLA-Escrow bank metadata. Monetize an API in about 30 minutes."
    link: /seller-quick-start
    linkText: Seller Quickstart
  - title: Buyers / Payer Agents
    details: "`npm i @pr402/client` or `cargo install pr402-client`. Both ship a `pr402-buy` CLI that delegates transaction assembly to pr402's `/build-*-payment-tx` endpoints — you sign once, the facilitator handles CU limits, token programs, and vault math."
    link: /quickstart-buyer
    linkText: Buyer Quickstart
  - title: Agent integration
    details: "Canonical runbook for sellers and buyers — schemes, headers, mint allowlists, pr402 vs the generic x402 spec. Pairs with OpenAPI 3.1 for precise schemas."
    link: /agent-integration
    linkText: Read runbook
---

## Status

The `exact` rail is GA on **Solana Mainnet** (`https://ipay.sh`) and **Devnet** (`https://preview.ipay.sh`); the same service is also served at `https://agent.pay402.me` / `https://preview.agent.pay402.me` (not deprecated).

`sla-escrow` is deployed on-chain; general availability for sellers/buyers depends on the `oracle_authority` chosen at funding. The open-source reference oracle is [`oracle-qa`](https://github.com/miraland-labs/oracle-qa). Operators and integrators should pin behavior to `GET /capabilities` and `GET /openapi.json` on the host they actually call.
