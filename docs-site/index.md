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
      text: Start here · Sellers
      link: /start-here
    - theme: alt
      text: Integrate your API
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
    details: "Orient (prerequisites, pick rail) → integrate (code samples) → optional quick reference. Fee tables and facilitator comparisons are in supplemental appendices."
    link: /start-here
    linkText: Start here · Sellers
  - title: Buyers / Payer Agents
    details: "`sla-escrow` escrow protection, facilitator tx builders, and open SDKs. Optional rehearsal on preview.ipay.sh. See how pr402 compares to CDP and pay.sh."
    link: /quickstart-buyer
    linkText: Buyer Quickstart
  - title: Choosing x402 on Solana
    details: "Facilitators (pr402 · CDP · x402.org) vs buyer tools (pay CLI). Two layers — compare the right one for your job."
    link: /pr402-vs-alternatives
    linkText: Facilitators & buyer tools
  - title: Agent integration
    details: "Canonical runbook for sellers and buyers — schemes, headers, mint allowlists, pr402 vs the generic x402 spec. Pairs with OpenAPI 3.1 for precise schemas."
    link: /agent-integration
    linkText: Read runbook
---

## Status

The `exact` rail is GA on **Solana Mainnet** (`https://ipay.sh`) and **Devnet** (`https://preview.ipay.sh`); the same service is also served at `https://agent.pay402.me` / `https://preview.agent.pay402.me` (not deprecated).

`sla-escrow` is deployed on-chain; general availability for sellers/buyers depends on the `oracle_authority` chosen at funding. Open-source reference oracles ship in the [`oracles/`](https://github.com/miraland-labs/oracles) workspace (three sibling profiles: api-quality, onchain-transfer, file-delivery). Operators and integrators should pin behavior to `GET /capabilities` and `GET /openapi.json` on the host they actually call.
